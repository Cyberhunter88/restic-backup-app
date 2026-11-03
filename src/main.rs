mod app;
mod config;
mod job;
mod restic;
mod schedule;
mod secrets;
mod smb;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "restic-Backup auf NAS (SMB)")]
struct Cli {
    /// Backup ohne GUI ausführen (für Zeitplan)
    #[arg(long)]
    run_backup: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.run_backup {
        let cfg = config::Config::load()?;
        job::run_backup(&cfg, false, &std::sync::atomic::AtomicBool::new(false), |m| println!("{m}"), |_| {})?;
        println!("Backup abgeschlossen.");
        return Ok(());
    }
    eframe::run_native(
        "restic Backup",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app::App::new()))),
    )
    .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))
}
