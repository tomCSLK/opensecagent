//! opensecagent – reports the installed software of a desktop computer to opensecmon.
//!
//! Copyright (C) 2026 Thomas Cieslik. Licensed under the GNU Affero General Public License, version 3 or later.

mod agent;
mod api;
mod code;
#[cfg(target_os = "linux")]
mod dialog;
mod http;
mod inventory;
#[cfg(target_os = "linux")]
mod legacy;
#[cfg(windows)]
#[path = "legacy_windows.rs"]
mod legacy;
mod log;
mod service;
mod store;
#[cfg(any(target_os = "linux", windows))]
mod texts;
#[cfg(windows)]
mod windialog;
#[cfg(windows)]
mod winsvc;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::agent::{Agent, Error};
use crate::store::Store;

#[derive(Parser)]
#[command(
    name = "opensecagent",
    version,
    about = "Reports the installed software of this computer to opensecmon"
)]
struct Cli {
    /// Folder of the agent's files (enrollment, token, state).
    #[arg(long, global = true, env = "OPENSECAGENT_DIR", value_name = "DIR")]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enroll this computer with the enrollment code from opensecmon (osa1_…), then send a first report.
    /// Without CODE the code is read from standard input, so it does not show in the process list.
    Enroll {
        code: Option<String>,
        /// Answer with one line of JSON, for the enrollment dialog.
        #[arg(long, hide = true)]
        for_dialog: bool,
        /// Read the code from this file and delete it (for the installer).
        #[arg(long, hide = true, value_name = "FILE", conflicts_with = "code")]
        code_file: Option<PathBuf>,
    },
    /// Send the inventory now.
    Report {
        /// Send the full list even if it did not change.
        #[arg(long)]
        full: bool,
    },
    /// Show the enrollment and ask opensecmon whether it knows this computer.
    Status,
    /// Print the report that would be sent, without sending anything.
    Inventory,
    /// Forget the enrollment: removes the token and the agent's files. The computer stays in opensecmon until it
    /// is deleted there.
    Uninstall,
    /// Show the enrollment dialog (from the start menu): takes the code from the clipboard or asks for it, then
    /// enrolls (Linux: through pkexec; Windows: as administrator, started by opensecagent-enroll.exe).
    #[cfg(any(target_os = "linux", windows))]
    EnrollDialog,
    /// Run as a service: report at the interval opensecmon asks for (started by systemd or the Windows service).
    Run,
    /// Set up the Windows service and start it (for the installer).
    #[cfg(windows)]
    #[command(hide = true)]
    ServiceInstall,
    /// Stop and remove the Windows service (for the installer).
    #[cfg(windows)]
    #[command(hide = true)]
    ServiceUninstall,
    /// Print the version.
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let default_dir = cli.dir.is_none();
    let store = cli.dir.map_or_else(Store::own, Store::new);
    // The service dispatcher blocks this thread and runs the service in another one, with its own runtime.
    #[cfg(windows)]
    if let Command::Run = cli.command {
        return winsvc::run(store);
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Command::Enroll {
        code,
        for_dialog: true,
        code_file,
    } = cli.command
    {
        return runtime.block_on(enroll_for_dialog(
            code,
            code_file,
            Agent::new(store),
            default_dir,
        ));
    }
    // The Windows dialog runs its own runtime in a thread while the window waits.
    #[cfg(windows)]
    if let Command::EnrollDialog = cli.command {
        drop(runtime);
        return windialog::run(store, default_dir);
    }
    match runtime.block_on(run(cli.command, Agent::new(store), default_dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command, agent: Agent, default_dir: bool) -> Result<(), Error> {
    match command {
        Command::Enroll {
            code, code_file, ..
        } => {
            let code = get_code(code, code_file)?;
            let enrolled = agent.enroll(&code, &inventory::hostname()).await?;
            println!(
                "Enrolled as \"{}\" with opensecmon at {}.",
                enrolled.device, enrolled.address
            );
            for path in remove_old_script(default_dir) {
                println!("Removed the old agent script: {}", path.display());
            }
            let first = report(&agent, false).await;
            if first.is_err() {
                println!("The first report did not work yet.");
            }
            first
        }
        Command::Report { full } => report(&agent, full).await,
        Command::Status => status(&agent).await,
        Command::Inventory => {
            let inventory = inventory::collect()?;
            let body = agent::report_body(&inventory, api::AgentInfo::this());
            let value: serde_json::Value = serde_json::from_slice(&body).expect("own JSON");
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("own JSON")
            );
            Ok(())
        }
        Command::Uninstall => {
            if agent.forget()? {
                println!("The enrollment was removed from this computer.");
                println!(
                    "To remove the computer from opensecmon as well, delete it on its page there."
                );
            } else {
                println!("This computer was not enrolled.");
            }
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::EnrollDialog => {
            dialog::run().map_err(Error::Dialog)?;
            Ok(())
        }
        // Handled in `main`.
        #[cfg(windows)]
        Command::EnrollDialog => Ok(()),
        Command::Run => service::run(agent, service::stop_signal()).await,
        #[cfg(windows)]
        Command::ServiceInstall => {
            winsvc::install()?;
            println!("The service {} is set up and running.", winsvc::NAME);
            Ok(())
        }
        #[cfg(windows)]
        Command::ServiceUninstall => {
            if winsvc::uninstall()? {
                println!("The service {} was removed.", winsvc::NAME);
            } else {
                println!("The service {} was not set up.", winsvc::NAME);
            }
            Ok(())
        }
        Command::Version => {
            println!("opensecagent {}", api::AGENT_VERSION);
            Ok(())
        }
    }
}

/// What an enrollment from a dialog came to.
struct DialogEnrollment {
    device: String,
    address: String,
    old_script_removed: bool,
    first_report: bool,
}

/// Enrolls, removes the old script and sends the first report, quietly (nothing on standard output).
async fn enroll_quietly(
    code: &str,
    agent: &Agent,
    default_dir: bool,
) -> Result<DialogEnrollment, Error> {
    let enrolled = agent.enroll(code, &inventory::hostname()).await?;
    let removed = remove_old_script(default_dir);
    let first_report = match collect(agent) {
        Ok(inventory) => agent.report(&inventory, false).await.is_ok(),
        Err(_) => false,
    };
    Ok(DialogEnrollment {
        device: enrolled.device,
        address: enrolled.address,
        old_script_removed: !removed.is_empty(),
        first_report,
    })
}

/// The enrollment for the dialog: one line of JSON on standard output, the exit code tells success.
async fn enroll_for_dialog(
    code: Option<String>,
    code_file: Option<PathBuf>,
    agent: Agent,
    default_dir: bool,
) -> ExitCode {
    let result = async {
        let code = get_code(code, code_file)?;
        enroll_quietly(&code, &agent, default_dir).await
    }
    .await;
    let line = match result {
        Ok(e) => serde_json::json!({
            "ok": true,
            "device": e.device,
            "address": e.address,
            "old_script_removed": e.old_script_removed,
            "first_report": e.first_report,
        }),
        Err(e) => serde_json::json!({
            "ok": false,
            "error": e.kind(),
            "message": e.to_string(),
        }),
    };
    let ok = line["ok"] == true;
    println!("{line}");
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// After an enrollment in the agent's own folder: removes the script opensecmon offered before, so the computer
/// does not report twice. A failure is only a warning.
fn remove_old_script(default_dir: bool) -> Vec<PathBuf> {
    #[cfg(any(target_os = "linux", windows))]
    if default_dir {
        match legacy::remove_old_script(&legacy::root()) {
            Ok(removed) => return removed.paths,
            Err(e) => log::note(format_args!(
                "warning: the old agent script could not be removed: {e}"
            )),
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = default_dir;
    Vec::new()
}

/// The code from the command line, from a file (deleted afterwards, also when it cannot be read), or from standard
/// input.
fn get_code(code: Option<String>, code_file: Option<PathBuf>) -> Result<String, Error> {
    if let Some(code) = code {
        return Ok(code);
    }
    let Some(path) = code_file else {
        return read_code();
    };
    let text = std::fs::File::open(&path).and_then(|f| {
        let mut text = String::new();
        f.take(4096).read_to_string(&mut text)?;
        Ok(text)
    });
    let _ = std::fs::remove_file(&path);
    text.map_err(|e| Error::Files(path, e))
}

/// The inventory for a report; if it cannot be read, that is kept as the last error for `status`.
fn collect(agent: &Agent) -> Result<inventory::Inventory, Error> {
    inventory::collect().map_err(|e| {
        let e = Error::Inventory(e);
        agent.note_error(&e);
        e
    })
}

async fn report(agent: &Agent, full: bool) -> Result<(), Error> {
    let inventory = collect(agent)?;
    let r = agent.report(&inventory, full).await?;
    let what = match (r.status.as_str(), r.short) {
        ("unchanged", true) => "unchanged since the last report (sent its hash only)".to_owned(),
        ("unchanged", false) => "sent, unchanged in opensecmon".to_owned(),
        _ => format!("sent ({} KB)", r.size.div_ceil(1024)),
    };
    println!(
        "Inventory {what}. Next report in {}.",
        duration(r.next_report_s)
    );
    for w in &r.warnings {
        println!("opensecmon skipped {} entries ({}).", w.count, w.code);
    }
    Ok(())
}

async fn status(agent: &Agent) -> Result<(), Error> {
    let s = agent.status().await?;
    println!("Computer:     {}", s.enrollment.device);
    println!("opensecmon:   https://{}", s.enrollment.address);
    if let Some(fp) = &s.enrollment.fingerprint {
        println!("Certificate:  {fp}");
    }
    println!("Enrolled:     {}", utc(s.enrollment.enrolled_at));
    println!(
        "Last report:  {}",
        s.state.last_report.map_or_else(|| "never".to_owned(), utc)
    );
    if let Some(e) = &s.state.last_error {
        println!("Last error:   {e}");
    }
    match s.remote {
        Ok(remote) => {
            println!(
                "Connection:   working, opensecmon knows this computer as \"{}\"",
                remote.device
            );
            if let Some(last) = &remote.last_report {
                println!("Last report seen by opensecmon: {last}");
            }
            println!("Report every: {}", duration(remote.next_report_s));
            Ok(())
        }
        Err(e) => {
            println!("Connection:   not working");
            Err(e)
        }
    }
}

/// Reads the enrollment code from standard input (pasted into a terminal, or piped).
fn read_code() -> Result<String, Error> {
    let stdin = std::io::stdin();
    let mut text = String::new();
    let result = if stdin.is_terminal() {
        eprint!("Enrollment code from opensecmon: ");
        stdin.read_line(&mut text)
    } else {
        stdin.take(4096).read_to_string(&mut text)
    };
    result.map_err(|e| Error::Files(PathBuf::from("<stdin>"), e))?;
    Ok(text)
}

/// `900` → `15 min`.
fn duration(seconds: u64) -> String {
    match seconds {
        s if s < 120 => format!("{s} s"),
        s if s < 2 * 3600 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

/// Seconds since 1970 as `2026-11-08 10:15 UTC`.
fn utc(seconds: u64) -> String {
    let t = date_time(seconds);
    format!("{} UTC", &t[..t.len() - 7])
}

/// Seconds since 1970 as `2026-11-08 10:15:42 UTC`.
fn date_time(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days to the civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn times_and_durations_read_well() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_762_596_900), "2025-11-08 10:15 UTC");
        assert_eq!(utc(951_825_600), "2000-02-29 12:00 UTC");
        assert_eq!(date_time(1_762_596_942), "2025-11-08 10:15:42 UTC");
        assert_eq!(duration(30), "30 s");
        assert_eq!(duration(900), "15 min");
        assert_eq!(duration(21_600), "6 h");
    }
}
