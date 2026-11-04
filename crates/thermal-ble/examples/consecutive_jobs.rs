//! Diagnostic: print several tiny strips over ONE BLE connection and report whether each
//! job is acknowledged. Uses ~3 mm of paper per job.
//!
//! cargo run -p thermal-ble --example consecutive_jobs -- [jobs=2] [idle_secs=0] [name=MiniX]

use std::time::{Duration, Instant};

use thermal_ble::{PrintOptions, Printer};
use thermal_proto::PrintJob;
use thermal_raster::Bitmap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let jobs: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(2);
    let idle: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let name = args.next().unwrap_or_else(|| "MiniX".into());

    let t0 = Instant::now();
    let mut printer = Printer::find(&name, Duration::from_secs(10)).await?;
    println!(
        "{:6.2}s connected to {}",
        t0.elapsed().as_secs_f32(),
        printer.name()
    );
    println!(
        "{:6.2}s status {}",
        t0.elapsed().as_secs_f32(),
        printer.status().await?
    );

    // A 24-row strip with a short dashed line, no feed: ~3 mm of paper per job.
    let strip = Bitmap::from_fn(384, 24, |x, y| (8..16).contains(&y) && (x / 16) % 2 == 0);
    let job = PrintJob {
        feed_dots: 0,
        ..Default::default()
    };
    for i in 1..=jobs {
        if i > 1 && idle > 0 {
            println!("{:6.2}s idle {idle}s…", t0.elapsed().as_secs_f32());
            tokio::time::sleep(Duration::from_secs(idle)).await;
            println!(
                "{:6.2}s connected={}",
                t0.elapsed().as_secs_f32(),
                printer.is_connected().await
            );
        }
        let start = Instant::now();
        let result = printer
            .print(&strip, &job, &PrintOptions::default(), |_| {})
            .await;
        match result {
            Ok(r) => println!(
                "{:6.2}s job {i}: OK in {:.2}s, acknowledged={}, status after={:?}",
                t0.elapsed().as_secs_f32(),
                start.elapsed().as_secs_f32(),
                r.acknowledged,
                r.status_after.map(|s| s.to_string())
            ),
            Err(e) => println!(
                "{:6.2}s job {i}: FAILED after {:.2}s: {e}",
                t0.elapsed().as_secs_f32(),
                start.elapsed().as_secs_f32()
            ),
        }
        println!(
            "{:6.2}s connected={}",
            t0.elapsed().as_secs_f32(),
            printer.is_connected().await
        );
    }
    printer.disconnect().await?;
    println!("{:6.2}s disconnected", t0.elapsed().as_secs_f32());
    Ok(())
}
