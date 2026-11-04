//! Transport-agnostic ("sans-I/O") print session.
//!
//! The session holds the hardware-verified print flow (status check → wake → enable →
//! chunked `GS v 0` → wait for `OK` → feed → end → status) but performs no I/O itself.
//! A driver repeatedly calls [`PrintSession::poll`] with a monotonic clock and carries out
//! the returned [`Action`], reporting results back:
//!
//! ```text
//! loop {
//!     match session.poll(now_ms) {
//!         Action::DrainReplies          => discard queued FF01 notifications
//!         Action::Write(bytes)          => write to FF02 (with response), then session.on_write_done()
//!         Action::Sleep { until_ms }    => sleep until then
//!         Action::AwaitReply { deadline_ms } => wait for the next FF01 notification:
//!                                          session.on_reply(bytes) or, at the deadline, session.on_timeout()
//!         Action::AwaitWrite            => (only if you poll before reporting a write)
//!         Action::Finished(report)      => done
//!         Action::Failed(err)           => stop
//!     }
//! }
//! ```
//!
//! FF03 frames can be passed to [`PrintSession::on_flow`] at any time to count credits.

use std::time::Duration;

use thermal_raster::Bitmap;

use crate::{FlowFrame, JobError, PrintJob, Status, cmd, is_ok, parse_flow};

/// Transport tuning. The defaults are the values verified on hardware.
#[derive(Clone, Copy, Debug)]
pub struct PrintOptions {
    /// Raster bytes per write (write-with-response).
    pub chunk_size: usize,
    /// Pause after each raster write.
    pub chunk_gap: Duration,
    /// How long to wait for the `OK` after the raster, on top of an allowance for
    /// rows still being printed.
    pub ok_timeout: Duration,
    /// Query status first and refuse to print on cover open / paper out / overheat.
    pub check_status: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            chunk_size: 128,
            chunk_gap: Duration::from_millis(20),
            ok_timeout: Duration::from_secs(10),
            check_status: true,
        }
    }
}

/// Where the session is, for progress displays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub stage: &'static str,
    pub sent: usize,
    pub total: usize,
}

/// What the driver should do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Discard FF01 notifications received so far, then poll again.
    DrainReplies,
    /// Write these bytes (write-with-response), then call [`PrintSession::on_write_done`].
    Write(Vec<u8>),
    /// Do nothing until this time, then poll again.
    Sleep {
        until_ms: u64,
    },
    /// Wait for the next FF01 notification ([`PrintSession::on_reply`]) or, at the
    /// deadline, call [`PrintSession::on_timeout`].
    AwaitReply {
        deadline_ms: u64,
    },
    /// A write is outstanding; report it with [`PrintSession::on_write_done`].
    AwaitWrite,
    Finished(SessionReport),
    Failed(SessionError),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("printer not ready: {0}")]
    NotReady(&'static str),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
}

/// Outcome of a completed session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionReport {
    pub bytes: usize,
    pub writes: usize,
    /// The printer acknowledged the raster with `OK`.
    pub acknowledged: bool,
    /// Credits granted on FF03 while the session ran.
    pub credits: u32,
    pub status_before: Option<Status>,
    pub status_after: Option<Status>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Await {
    StatusBefore,
    Ok,
    TrailingOk,
    StatusAfter,
}

#[derive(Clone, Debug)]
enum Op {
    Drain,
    Write {
        data: Vec<u8>,
        stage: &'static str,
        gap: bool,
        payload: bool,
    },
    Await {
        kind: Await,
        timeout_ms: u64,
        stage: &'static str,
    },
}

#[derive(Clone, Debug)]
enum State {
    Ready,
    Writing {
        len: usize,
        gap: bool,
        payload: bool,
    },
    Sleeping {
        until_ms: Option<u64>,
    },
    Awaiting {
        kind: Await,
        deadline_ms: Option<u64>,
        timeout_ms: u64,
    },
    Finished,
    Failed(SessionError),
}

/// See the [module docs](self).
#[derive(Clone, Debug)]
pub struct PrintSession {
    ops: Vec<Op>,
    next: usize,
    state: State,
    stage: &'static str,
    gap_ms: u64,
    total: usize,
    sent: usize,
    report: SessionReport,
}

const REPLY_TIMEOUT_MS: u64 = 2_000;
const TRAILING_OK_MS: u64 = 1_500;

impl PrintSession {
    pub fn new(bitmap: &Bitmap, job: &PrintJob, opts: &PrintOptions) -> Result<Self, JobError> {
        let segments = job.segments(bitmap)?;
        let total = segments.iter().map(|s| s.data.len()).sum();
        let write = |data: &[u8], stage, gap, payload| Op::Write {
            data: data.to_vec(),
            stage,
            gap,
            payload,
        };
        let mut ops = Vec::new();

        if opts.check_status {
            ops.push(Op::Drain);
            ops.push(write(&cmd::QUERY_STATUS, "checking printer", false, false));
            ops.push(Op::Await {
                kind: Await::StatusBefore,
                timeout_ms: REPLY_TIMEOUT_MS,
                stage: "checking printer",
            });
        }
        for seg in &segments {
            if seg.expect_ok {
                ops.push(Op::Drain);
            }
            let chunk = if seg.chunked {
                opts.chunk_size.max(1)
            } else {
                seg.data.len().max(1)
            };
            for piece in seg.data.chunks(chunk) {
                ops.push(write(piece, seg.label, seg.chunked, true));
            }
            if seg.expect_ok {
                // Allow for rows still in the printer's buffer (~20 mm/s worst case).
                let rows = bitmap.height() as u64;
                let timeout_ms = opts.ok_timeout.as_millis() as u64 + rows * 1000 / 160;
                ops.push(Op::Await {
                    kind: Await::Ok,
                    timeout_ms,
                    stage: "printing",
                });
            }
        }
        ops.push(Op::Await {
            kind: Await::TrailingOk,
            timeout_ms: TRAILING_OK_MS,
            stage: "finishing",
        });
        ops.push(Op::Drain);
        ops.push(write(&cmd::QUERY_STATUS, "finishing", false, false));
        ops.push(Op::Await {
            kind: Await::StatusAfter,
            timeout_ms: REPLY_TIMEOUT_MS,
            stage: "finishing",
        });

        Ok(Self {
            ops,
            next: 0,
            state: State::Ready,
            stage: "starting",
            gap_ms: opts.chunk_gap.as_millis() as u64,
            total,
            sent: 0,
            report: SessionReport {
                bytes: total,
                ..Default::default()
            },
        })
    }

    /// Current stage and payload bytes written (status queries are not counted).
    pub fn progress(&self) -> Progress {
        let stage = if matches!(self.state, State::Finished) {
            "done"
        } else {
            self.stage
        };
        Progress {
            stage,
            sent: self.sent,
            total: self.total,
        }
    }

    /// Advance the session. `now_ms` is any monotonic clock in milliseconds.
    pub fn poll(&mut self, now_ms: u64) -> Action {
        loop {
            match &mut self.state {
                State::Finished => return Action::Finished(self.report.clone()),
                State::Failed(e) => return Action::Failed(e.clone()),
                State::Writing { .. } => return Action::AwaitWrite,
                State::Sleeping { until_ms } => {
                    let until = *until_ms.get_or_insert(now_ms + self.gap_ms);
                    if now_ms >= until {
                        self.state = State::Ready;
                        continue;
                    }
                    return Action::Sleep { until_ms: until };
                }
                State::Awaiting {
                    kind,
                    deadline_ms,
                    timeout_ms,
                } => {
                    let deadline = *deadline_ms.get_or_insert(now_ms + *timeout_ms);
                    if now_ms >= deadline {
                        let kind = *kind;
                        self.timed_out(kind);
                        continue;
                    }
                    return Action::AwaitReply {
                        deadline_ms: deadline,
                    };
                }
                State::Ready => {
                    let Some(op) = self.ops.get(self.next).cloned() else {
                        self.state = State::Finished;
                        continue;
                    };
                    self.next += 1;
                    match op {
                        Op::Drain => return Action::DrainReplies,
                        Op::Write {
                            data,
                            stage,
                            gap,
                            payload,
                        } => {
                            self.stage = stage;
                            self.state = State::Writing {
                                len: data.len(),
                                gap,
                                payload,
                            };
                            return Action::Write(data);
                        }
                        Op::Await {
                            kind,
                            timeout_ms,
                            stage,
                        } => {
                            self.stage = stage;
                            self.state = State::Awaiting {
                                kind,
                                deadline_ms: Some(now_ms + timeout_ms),
                                timeout_ms,
                            };
                        }
                    }
                }
            }
        }
    }

    /// The write returned by the last [`Action::Write`] was acknowledged.
    pub fn on_write_done(&mut self) {
        if let State::Writing { len, gap, payload } = self.state {
            self.report.writes += 1;
            if payload {
                self.sent += len;
            }
            self.state = if gap && self.gap_ms > 0 {
                State::Sleeping { until_ms: None }
            } else {
                State::Ready
            };
        }
    }

    /// A notification arrived on FF01 while awaiting a reply.
    pub fn on_reply(&mut self, reply: &[u8]) {
        let State::Awaiting { kind, .. } = self.state else {
            return;
        };
        let status = || Status(reply.last().copied().unwrap_or(0));
        match kind {
            Await::StatusBefore => {
                let s = status();
                self.report.status_before = Some(s);
                self.state = match s.problem() {
                    Some(problem) => State::Failed(SessionError::NotReady(problem)),
                    None => State::Ready,
                };
            }
            Await::Ok => {
                if is_ok(reply) {
                    self.report.acknowledged = true;
                    self.state = State::Ready;
                }
                // Anything else: keep waiting until the same deadline.
            }
            Await::TrailingOk => self.state = State::Ready,
            Await::StatusAfter => {
                self.report.status_after = Some(status());
                self.state = State::Ready;
            }
        }
    }

    /// The deadline of the last [`Action::AwaitReply`] passed without a reply.
    pub fn on_timeout(&mut self) {
        if let State::Awaiting { kind, .. } = self.state {
            self.timed_out(kind);
        }
    }

    /// A frame arrived on FF03 (flow control). Safe to call at any time.
    pub fn on_flow(&mut self, frame: &[u8]) {
        if let FlowFrame::Credits(n) = parse_flow(frame) {
            self.report.credits += n as u32;
        }
    }

    fn timed_out(&mut self, kind: Await) {
        self.state = match kind {
            Await::StatusBefore => State::Failed(SessionError::Timeout("printer status")),
            Await::Ok => State::Failed(SessionError::Timeout("the printer's OK after the image")),
            Await::TrailingOk | Await::StatusAfter => State::Ready,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulated printer that behaves like the hardware transcript: answers status
    /// queries with `status`, says `OK` once the whole raster has arrived (unless
    /// `silent`) and after the end-of-job command. Returns every write.
    fn run(bitmap: &Bitmap, status: u8, silent: bool) -> (Vec<Vec<u8>>, Action) {
        let mut s =
            PrintSession::new(bitmap, &PrintJob::default(), &PrintOptions::default()).unwrap();
        let raster_len = 8 + bitmap.as_bytes().len();
        let (mut now, mut raster_seen, mut in_raster) = (0u64, 0usize, false);
        let (mut writes, mut pending): (Vec<Vec<u8>>, Vec<Vec<u8>>) = (vec![], vec![]);
        loop {
            match s.poll(now) {
                Action::DrainReplies => pending.clear(),
                Action::Write(data) => {
                    if data == cmd::QUERY_STATUS {
                        pending.push(vec![status]);
                    } else if data == cmd::ENABLE {
                        in_raster = true;
                    } else if data == cmd::END_JOB {
                        pending.push(b"OK".to_vec());
                    } else if in_raster {
                        raster_seen += data.len();
                        if raster_seen == raster_len {
                            in_raster = false;
                            if !silent {
                                pending.push(b"OK".to_vec());
                            }
                        }
                    }
                    writes.push(data);
                    s.on_flow(&[0x01, 0x01]);
                    s.on_write_done();
                }
                Action::Sleep { until_ms } => now = until_ms,
                Action::AwaitReply { deadline_ms } => {
                    if pending.is_empty() {
                        now = deadline_ms;
                        s.on_timeout();
                    } else {
                        let r = pending.remove(0);
                        s.on_reply(&r);
                    }
                }
                Action::AwaitWrite => unreachable!(),
                end @ (Action::Finished(_) | Action::Failed(_)) => return (writes, end),
            }
        }
    }

    #[test]
    fn replays_the_verified_hardware_transcript() {
        // RESEARCH.md §5.3: 384 × 200 test image → 82 writes, OK after the raster.
        let bmp = Bitmap::from_fn(384, 200, |x, y| (x * y) % 5 == 0);
        let (writes, end) = run(&bmp, 0x00, false);
        assert_eq!(writes.len(), 82);
        assert_eq!(writes[0], cmd::QUERY_STATUS);
        assert_eq!(writes[1], cmd::WAKE);
        assert_eq!(writes[2], cmd::ENABLE);
        assert_eq!(
            writes[3][..8],
            [0x1D, 0x76, 0x30, 0x00, 0x30, 0x00, 0xC8, 0x00]
        );
        assert!(writes[3..79].iter().all(|w| w.len() <= 128));
        assert_eq!(writes[79], cmd::feed(80));
        assert_eq!(writes[80], cmd::END_JOB);
        assert_eq!(writes[81], cmd::QUERY_STATUS);
        let Action::Finished(report) = end else {
            panic!("{end:?}")
        };
        assert!(report.acknowledged);
        assert_eq!(report.bytes, 12 + 4 + 8 + 9600 + 3 + 4);
        assert_eq!(report.writes, 82);
        assert_eq!(report.credits, 82);
        assert_eq!(report.status_before, Some(Status(0)));
        assert_eq!(report.status_after, Some(Status(0)));
    }

    #[test]
    fn refuses_to_print_without_paper() {
        let (writes, end) = run(&Bitmap::new(384, 10), 0x04, false);
        assert_eq!(writes, vec![cmd::QUERY_STATUS.to_vec()]);
        assert_eq!(end, Action::Failed(SessionError::NotReady("out of paper")));
    }

    #[test]
    fn fails_when_the_raster_is_never_acknowledged() {
        let (_, end) = run(&Bitmap::new(384, 10), 0x00, true);
        assert_eq!(
            end,
            Action::Failed(SessionError::Timeout("the printer's OK after the image"))
        );
    }

    #[test]
    fn progress_counts_payload_only() {
        let bmp = Bitmap::new(384, 4);
        let mut s =
            PrintSession::new(&bmp, &PrintJob::default(), &PrintOptions::default()).unwrap();
        assert_eq!(s.progress().sent, 0);
        assert!(matches!(s.poll(0), Action::DrainReplies));
        assert_eq!(s.poll(0), Action::Write(cmd::QUERY_STATUS.to_vec()));
        s.on_write_done();
        assert_eq!(s.progress().sent, 0, "status query is not payload");
        assert!(matches!(
            s.poll(0),
            Action::AwaitReply { deadline_ms: 2000 }
        ));
        s.on_reply(&[0]);
        assert_eq!(s.poll(1), Action::Write(cmd::WAKE.to_vec()));
        s.on_write_done();
        assert_eq!(s.progress().sent, 12);
    }
}
