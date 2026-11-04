//! Bluetooth LE transport for LuckPrinter-family printers (Seznik MiniX / LuckJingle S1).
//!
//! The printer exposes service `FF00`: write commands to `FF02`, replies arrive on
//! `FF01`, flow-control credits on `FF03`. No pairing is needed. See `RESEARCH.md` §2.2 / §5.
//!
//! ```no_run
//! # async fn demo(bitmap: thermal_raster::Bitmap) -> Result<(), thermal_ble::Error> {
//! use std::time::Duration;
//! let mut printer = thermal_ble::Printer::find("MiniX", Duration::from_secs(10)).await?;
//! printer.print(&bitmap, &Default::default(), &Default::default(), |_| {}).await?;
//! printer.disconnect().await?;
//! # Ok(()) }
//! ```
//!
//! macOS: the process (or the terminal it runs in) needs Bluetooth permission. A bundled
//! `.app` must declare `NSBluetoothAlwaysUsageDescription` in its `Info.plist`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{
    Central, CentralState, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use thermal_proto::{Action, FlowFrame, PrintJob, PrintSession, SessionError, Status, cmd, gatt};
use thermal_raster::Bitmap;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

pub use thermal_proto::{Density, JobError, PrintOptions, Progress};

pub const SERVICE: Uuid = uuid_from_u16(gatt::SERVICE);
/// Command / data input.
pub const CHAR_WRITE: Uuid = uuid_from_u16(gatt::WRITE);
/// Replies (`OK`, query results).
pub const CHAR_REPLY: Uuid = uuid_from_u16(gatt::REPLY);
/// Flow-control frames.
pub const CHAR_FLOW: Uuid = uuid_from_u16(gatt::FLOW);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bluetooth: {0}")]
    Bluetooth(#[from] btleplug::Error),
    #[error("no Bluetooth adapter found")]
    NoAdapter,
    #[error("Bluetooth is off or unavailable")]
    BluetoothOff,
    #[error("no printer whose name contains \"{0}\" found (is it switched on?)")]
    NotFound(String),
    #[error("printer is missing characteristic {0}")]
    MissingCharacteristic(Uuid),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("printer not ready: {0}")]
    NotReady(String),
    #[error(transparent)]
    Job(#[from] JobError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Outcome of a print.
#[derive(Clone, Debug)]
pub struct PrintReport {
    pub bytes: usize,
    pub writes: usize,
    pub elapsed: Duration,
    /// The printer acknowledged the raster with `OK`.
    pub acknowledged: bool,
    /// Flow-control credits granted on FF03 during the job.
    pub credits: u32,
    pub status_after: Option<Status>,
}

/// Identity and state read from the printer.
#[derive(Clone, Debug, Default)]
pub struct PrinterInfo {
    pub name: String,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub battery: Option<u8>,
    pub status: Option<Status>,
}

/// A connected printer.
pub struct Printer {
    peripheral: Peripheral,
    write: Characteristic,
    name: String,
    replies: mpsc::UnboundedReceiver<Vec<u8>>,
    credits: Arc<AtomicU32>,
    listener: tokio::task::JoinHandle<()>,
}

/// A likely printer seen while scanning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundPrinter {
    /// Advertised name, e.g. `Seznik MiniX_XXXX_LE`. Unique per printer; use it to
    /// reconnect with [`Printer::find`].
    pub name: String,
}

async fn powered_adapter() -> Result<Adapter> {
    let manager = Manager::new().await?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or(Error::NoAdapter)?;
    // CoreBluetooth reports "unknown" briefly after start-up.
    let ready = Instant::now();
    while adapter.adapter_state().await? != CentralState::PoweredOn {
        if ready.elapsed() > Duration::from_secs(3) {
            return Err(Error::BluetoothOff);
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(adapter)
}

fn advertised_name(props: btleplug::api::PeripheralProperties) -> Option<String> {
    props
        .local_name
        .or(props.advertisement_name)
        .filter(|n| !n.is_empty())
}

/// Scan for nearby printers for `duration`, calling `on_found` once for each new one.
/// Devices are recognised by name ([`thermal_proto::gatt::looks_like_printer`]) or by
/// advertising the printer service.
pub async fn scan_printers(
    duration: Duration,
    mut on_found: impl FnMut(&FoundPrinter),
) -> Result<Vec<FoundPrinter>> {
    let adapter = powered_adapter().await?;
    adapter.start_scan(ScanFilter::default()).await?;
    let deadline = Instant::now() + duration;
    let mut found: Vec<FoundPrinter> = Vec::new();
    while Instant::now() < deadline {
        for p in adapter.peripherals().await? {
            let Some(props) = p.properties().await? else {
                continue;
            };
            let advertises_service = props.services.contains(&SERVICE);
            let Some(name) = advertised_name(props) else {
                continue;
            };
            if (advertises_service || gatt::looks_like_printer(&name))
                && !found.iter().any(|f| f.name == name)
            {
                let printer = FoundPrinter { name };
                on_found(&printer);
                found.push(printer);
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    adapter.stop_scan().await.ok();
    Ok(found)
}

impl Printer {
    /// Scan for a printer whose advertised name contains `name_contains`
    /// (case-insensitive; a full name from [`scan_printers`] matches exactly that
    /// printer), connect and subscribe. Must run inside a Tokio runtime.
    pub async fn find(name_contains: &str, scan_timeout: Duration) -> Result<Printer> {
        let adapter = powered_adapter().await?;
        let needle = name_contains.to_lowercase();
        adapter.start_scan(ScanFilter::default()).await?;
        let deadline = Instant::now() + scan_timeout;
        let found = loop {
            let mut hit = None;
            for p in adapter.peripherals().await? {
                if let Some(name) = p.properties().await?.and_then(advertised_name)
                    && name.to_lowercase().contains(&needle)
                {
                    hit = Some((p, name));
                    break;
                }
            }
            if let Some(hit) = hit {
                break Some(hit);
            }
            if Instant::now() >= deadline {
                break None;
            }
            sleep(Duration::from_millis(250)).await;
        };
        adapter.stop_scan().await.ok();
        let (peripheral, name) = found.ok_or_else(|| Error::NotFound(name_contains.to_string()))?;
        Self::connect(peripheral, name).await
    }

    async fn connect(peripheral: Peripheral, name: String) -> Result<Printer> {
        if !peripheral.is_connected().await? {
            timeout(Duration::from_secs(15), peripheral.connect())
                .await
                .map_err(|_| Error::Timeout("connection"))??;
        }
        peripheral.discover_services().await?;
        let chars = peripheral.characteristics();
        let find = |uuid: Uuid| {
            chars
                .iter()
                .find(|c| c.uuid == uuid && c.service_uuid == SERVICE)
                .cloned()
                .ok_or(Error::MissingCharacteristic(uuid))
        };
        let write = find(CHAR_WRITE)?;
        let reply = find(CHAR_REPLY)?;
        let flow = find(CHAR_FLOW)?;

        let mut stream = peripheral.notifications().await?;
        peripheral.subscribe(&reply).await?;
        peripheral.subscribe(&flow).await?;

        let (tx, replies) = mpsc::unbounded_channel();
        let credits = Arc::new(AtomicU32::new(0));
        let credit_count = credits.clone();
        let listener = tokio::spawn(async move {
            while let Some(n) = stream.next().await {
                if n.uuid == CHAR_REPLY {
                    if tx.send(n.value).is_err() {
                        break;
                    }
                } else if n.uuid == CHAR_FLOW
                    && let FlowFrame::Credits(c) = thermal_proto::parse_flow(&n.value)
                {
                    credit_count.fetch_add(c as u32, Ordering::Relaxed);
                }
            }
        });

        let mut printer = Printer {
            peripheral,
            write,
            name,
            replies,
            credits,
            listener,
        };
        sleep(Duration::from_millis(300)).await; // let connect-time frames arrive
        printer.drain();
        Ok(printer)
    }

    /// Advertised name, e.g. `Seznik MiniX_XXXX_LE`.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    pub async fn disconnect(self) -> Result<()> {
        self.listener.abort();
        self.peripheral.disconnect().await?;
        Ok(())
    }

    fn drain(&mut self) {
        while self.replies.try_recv().is_ok() {}
    }

    async fn write(&self, data: &[u8]) -> Result<()> {
        timeout(
            Duration::from_secs(5),
            self.peripheral
                .write(&self.write, data, WriteType::WithResponse),
        )
        .await
        .map_err(|_| Error::Timeout("BLE write acknowledgement"))??;
        Ok(())
    }

    async fn next_reply(&mut self, wait: Duration) -> Option<Vec<u8>> {
        timeout(wait, self.replies.recv()).await.ok().flatten()
    }

    /// Send a query command and return the first FF01 reply.
    pub async fn query(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        self.drain();
        self.write(command).await?;
        self.next_reply(Duration::from_secs(2))
            .await
            .ok_or(Error::Timeout("printer reply"))
    }

    pub async fn status(&mut self) -> Result<Status> {
        let r = self.query(&cmd::QUERY_STATUS).await?;
        Ok(Status(r.last().copied().unwrap_or(0)))
    }

    /// Model, firmware, battery and status. Fields the firmware doesn't answer stay `None`.
    pub async fn info(&mut self) -> Result<PrinterInfo> {
        let text = |r: Result<Vec<u8>>| {
            r.ok()
                .map(|v| thermal_proto::parse_text(&v))
                .filter(|s| !s.is_empty())
        };
        Ok(PrinterInfo {
            name: self.name.clone(),
            model: text(self.query(&cmd::QUERY_MODEL).await),
            firmware: text(self.query(&cmd::QUERY_FIRMWARE).await),
            battery: self
                .query(&cmd::QUERY_BATTERY)
                .await
                .ok()
                .and_then(|r| thermal_proto::parse_battery(&r)),
            status: Some(self.status().await?),
        })
    }

    /// Print a bitmap (≤ 384 dots wide; narrower images are centred).
    ///
    /// Drives a [`PrintSession`], which holds the hardware-verified flow; this method
    /// only performs the BLE I/O it asks for.
    pub async fn print(
        &mut self,
        bitmap: &Bitmap,
        job: &PrintJob,
        opts: &PrintOptions,
        mut progress: impl FnMut(Progress),
    ) -> Result<PrintReport> {
        let mut session = PrintSession::new(bitmap, job, opts)?;
        let start = Instant::now();
        let credits_before = self.credits.load(Ordering::Relaxed);
        let now_ms = || start.elapsed().as_millis() as u64;
        progress(session.progress());

        loop {
            match session.poll(now_ms()) {
                Action::DrainReplies => self.drain(),
                Action::Write(data) => {
                    self.write(&data).await?;
                    session.on_write_done();
                    progress(session.progress());
                }
                Action::Sleep { until_ms } => {
                    sleep(Duration::from_millis(until_ms.saturating_sub(now_ms()))).await;
                }
                Action::AwaitReply { deadline_ms } => {
                    let wait = Duration::from_millis(deadline_ms.saturating_sub(now_ms()));
                    match self.next_reply(wait).await {
                        Some(reply) => session.on_reply(&reply),
                        None => session.on_timeout(),
                    }
                    progress(session.progress());
                }
                // This driver always reports writes before polling again.
                Action::AwaitWrite => unreachable!("write reported synchronously"),
                Action::Finished(report) => {
                    progress(session.progress());
                    return Ok(PrintReport {
                        bytes: report.bytes,
                        writes: report.writes,
                        elapsed: start.elapsed(),
                        acknowledged: report.acknowledged,
                        credits: self
                            .credits
                            .load(Ordering::Relaxed)
                            .saturating_sub(credits_before),
                        status_after: report.status_after,
                    });
                }
                Action::Failed(SessionError::NotReady(why)) => {
                    return Err(Error::NotReady(why.to_string()));
                }
                Action::Failed(SessionError::Timeout(what)) => return Err(Error::Timeout(what)),
            }
        }
    }
}
