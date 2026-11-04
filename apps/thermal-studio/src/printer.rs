//! Background printer worker. Owns a Tokio runtime on its own thread so the UI never
//! blocks; talks to the UI through channels.
//!
//! Like the vendor app, the printer is only connected while a job or status check runs:
//! connect → work → disconnect. That keeps it free for other devices and its LED idle.

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use thermal_ble::{FoundPrinter, PrintOptions, PrintReport, Printer, PrinterInfo};
use thermal_proto::PrintJob;
use thermal_raster::Bitmap;
use tokio::sync::mpsc;

const SCAN_DURATION: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Command {
    /// Search for nearby printers.
    Scan,
    /// Connect to the named printer, read its info, disconnect.
    Check { name: String },
    /// Connect to the named printer, print, read its info, disconnect.
    Print {
        bitmap: Bitmap,
        job: PrintJob,
        name: String,
    },
}

pub enum Event {
    Found(FoundPrinter),
    ScanDone,
    Busy(&'static str),
    Progress {
        stage: &'static str,
        fraction: f32,
    },
    Info(PrinterInfo),
    Printed(PrintReport),
    Failed(String),
    /// The command finished; the printer is disconnected.
    Idle,
}

pub struct PrinterWorker {
    tx: mpsc::UnboundedSender<Command>,
    rx: std_mpsc::Receiver<Event>,
}

impl PrinterWorker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, commands) = mpsc::unbounded_channel();
        let (events, rx) = std_mpsc::channel();
        std::thread::Builder::new()
            .name("printer".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(run(commands, Emitter { events, ctx }));
            })
            .expect("spawn printer thread");
        Self { tx, rx }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct Emitter {
    events: std_mpsc::Sender<Event>,
    ctx: egui::Context,
}

impl Emitter {
    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
        self.ctx.request_repaint();
    }
}

async fn run(mut commands: mpsc::UnboundedReceiver<Command>, out: Emitter) {
    while let Some(command) = commands.recv().await {
        let result = match command {
            Command::Scan => {
                thermal_ble::scan_printers(SCAN_DURATION, |p| out.emit(Event::Found(p.clone())))
                    .await
                    .map(|_| ())
            }
            Command::Check { name } => check(&name, &out).await,
            Command::Print { bitmap, job, name } => print(&bitmap, &job, &name, &out).await,
        };
        if let Err(e) = result {
            out.emit(Event::Failed(e.to_string()));
        }
        out.emit(Event::ScanDone);
        out.emit(Event::Idle);
    }
}

async fn connect(name: &str, out: &Emitter) -> thermal_ble::Result<Printer> {
    out.emit(Event::Busy("Connecting…"));
    Printer::find(name, CONNECT_TIMEOUT).await
}

async fn check(name: &str, out: &Emitter) -> thermal_ble::Result<()> {
    let mut printer = connect(name, out).await?;
    let info = printer.info().await;
    let _ = printer.disconnect().await;
    out.emit(Event::Info(info?));
    Ok(())
}

async fn print(
    bitmap: &Bitmap,
    job: &PrintJob,
    name: &str,
    out: &Emitter,
) -> thermal_ble::Result<()> {
    let mut printer = connect(name, out).await?;
    let mut last = Instant::now() - Duration::from_secs(1);
    let result = printer
        .print(bitmap, job, &PrintOptions::default(), |p| {
            if last.elapsed() > Duration::from_millis(50) || p.stage == "done" {
                last = Instant::now();
                let fraction = p.sent as f32 / p.total.max(1) as f32;
                out.emit(Event::Progress {
                    stage: p.stage,
                    fraction,
                });
            }
        })
        .await;
    let info = match &result {
        Ok(_) => printer.info().await.ok(),
        Err(_) => None,
    };
    let _ = printer.disconnect().await;
    out.emit(Event::Printed(result?));
    if let Some(info) = info {
        out.emit(Event::Info(info));
    }
    Ok(())
}
