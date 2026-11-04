//! Workshop downloads for people without the Steam client. steamcmd runs in a terminal of
//! its own: it asks for the password and the Steam Guard code there and gets the answers the
//! same way, so they never land on its command line, on disk or in a log. steamcmd keeps its
//! own sign-in afterwards; this app keeps only the account name.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use apolline_wall_protocol::install::{self, Need};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError};

use crate::config;

const APP: &str = "431960";
const PROGRAM: &str = "steamcmd";
/// Another program that takes steamcmd's arguments, as the tests use.
const OVERRIDE: &str = "APOLLINE_WALL_STEAMCMD";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// Signs in and checks that the account owns Wallpaper Engine.
    SignIn,
    Download(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    Password,
    Code,
    /// The sign-in waits for a tap in the Steam mobile app.
    Approval,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    Asked(Prompt),
    /// Bytes downloaded so far.
    Progress(u64),
    Done(Result<(), String>),
}

pub enum Input {
    Answer(String),
    Cancel,
}

pub fn installed() -> bool {
    std::env::var_os(OVERRIDE).is_some()
        || std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(PROGRAM).is_file()))
}

/// Steam account names are letters, digits and underscores; anything else could also be
/// read as one of steamcmd's commands.
pub fn valid_account(name: &str) -> bool {
    (3..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// steamcmd's install folder for this app. It downloads into `downloads` there and moves a
/// finished item into `content` in one step, so a half-done download never shows up.
fn root() -> PathBuf {
    config::data_dir().join("steamcmd")
}

/// Where downloaded Workshop items are, each in a folder named by its id.
pub fn content() -> PathBuf {
    root().join("steamapps/workshop/content").join(APP)
}

/// The update time of each item downloaded here, as steamcmd recorded it.
pub fn versions() -> HashMap<String, u64> {
    let manifest = root()
        .join("steamapps/workshop")
        .join(format!("appworkshop_{APP}.acf"));
    let recorded = std::fs::read_to_string(manifest)
        .map(|acf| times_in(&acf))
        .unwrap_or_default();
    std::fs::read_dir(content())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .map(|id| {
            let time = recorded.get(&id).copied().unwrap_or(0);
            (id, time)
        })
        .collect()
}

/// `"timeupdated"` per item in an appworkshop manifest; the first block an item appears in
/// is the installed one.
fn times_in(acf: &str) -> HashMap<String, u64> {
    let mut times = HashMap::new();
    let mut item: Option<String> = None;
    for line in acf.lines() {
        let words: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        match words.as_slice() {
            [id] if id.chars().all(|c| c.is_ascii_digit()) => item = Some((*id).to_owned()),
            ["timeupdated", time] => {
                if let (Some(id), Ok(time)) = (&item, time.parse()) {
                    times.entry(id.clone()).or_insert(time);
                }
            }
            _ => {}
        }
    }
    times
}

/// Which program runs and where it installs.
struct Setup {
    program: std::ffi::OsString,
    root: PathBuf,
}

impl Setup {
    fn usual() -> Self {
        Setup {
            program: std::env::var_os(OVERRIDE).unwrap_or_else(|| PROGRAM.into()),
            root: root(),
        }
    }
}

/// Runs a job on a thread of its own. What steamcmd asks and how it ends comes back on the
/// returned channel; answers and a cancel go in through `input`.
pub fn start(
    job: Job,
    account: String,
    input: UnboundedReceiver<Input>,
) -> UnboundedReceiver<Heard> {
    start_with(Setup::usual(), job, account, input)
}

fn start_with(
    setup: Setup,
    job: Job,
    account: String,
    input: UnboundedReceiver<Input>,
) -> UnboundedReceiver<Heard> {
    let (heard, hearing) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let result = run(&setup, &job, &account, input, &heard);
        let _ = heard.send(Heard::Done(result));
    });
    hearing
}

fn run(
    setup: &Setup,
    job: &Job,
    account: &str,
    mut input: UnboundedReceiver<Input>,
    heard: &UnboundedSender<Heard>,
) -> Result<(), String> {
    if !valid_account(account) {
        return Err(format!("“{account}” isn't a Steam account name."));
    }
    if let Job::Download(id) = job
        && (id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(format!("“{id}” isn't a Workshop item."));
    }
    let root = &setup.root;
    std::fs::create_dir_all(root).map_err(|err| format!("{}: {err}", root.display()))?;
    let (master, slave) = terminal()?;
    let mut child = command(setup, job, account, &slave)?
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => format!(
                "Downloads without the Steam client need steamcmd: {}.",
                install::hint(Need::Steamcmd)
            ),
            _ => format!("steamcmd: {err}"),
        })?;
    drop(slave);
    let pid = child.id() as i32;
    let mut writer = File::from(master.try_clone().map_err(|err| err.to_string())?);
    let reader = {
        let heard = heard.clone();
        let mut terminal = File::from(master);
        std::thread::spawn(move || listen(&mut terminal, &heard))
    };
    let mut cancelled = false;
    let mut measured = Instant::now();
    while !reader.is_finished() {
        match input.try_recv() {
            Ok(Input::Answer(answer)) => {
                let _ = writer.write_all(format!("{answer}\n").as_bytes());
            }
            Ok(Input::Cancel) | Err(TryRecvError::Disconnected) if !cancelled => {
                cancelled = true;
                unsafe { libc::kill(-pid, libc::SIGTERM) };
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
        if let Job::Download(id) = job
            && measured.elapsed() >= Duration::from_secs(1)
        {
            measured = Instant::now();
            let downloading = root.join("steamapps/workshop/downloads").join(APP).join(id);
            let _ = heard.send(Heard::Progress(size_of(&downloading)));
        }
    }
    let reading = reader
        .join()
        .map_err(|_| "steamcmd's output broke".to_owned())?;
    let status = child.wait().map_err(|err| err.to_string())?;
    if cancelled {
        return Err("Stopped.".into());
    }
    let content = root.join("steamapps/workshop/content").join(APP);
    reading.verdict(job, status.success(), &content)
}

fn command(setup: &Setup, job: &Job, account: &str, slave: &OwnedFd) -> Result<Command, String> {
    let mut command = Command::new(&setup.program);
    command
        .args(["+@ShutdownOnFailedCommand", "1", "+force_install_dir"])
        .arg(&setup.root)
        .args(["+login", account]);
    match job {
        Job::SignIn => command.args(["+licenses_for_app", APP]),
        Job::Download(id) => command.args(["+workshop_download_item", APP, id]),
    };
    command.arg("+quit");
    let side = || {
        slave
            .try_clone()
            .map(Stdio::from)
            .map_err(|err| err.to_string())
    };
    command.stdin(side()?).stdout(side()?).stderr(side()?);
    // Its own session with the terminal as its controlling one, as a shell would give it, and
    // gone when the app goes.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            #[cfg(target_os = "linux")]
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    Ok(command)
}

/// A pseudo-terminal that doesn't echo, so an answer never comes back in what is read.
fn terminal() -> Result<(OwnedFd, OwnedFd), String> {
    let (mut master, mut slave) = (-1, -1);
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if opened != 0 {
        return Err(format!(
            "a terminal for steamcmd: {}",
            std::io::Error::last_os_error()
        ));
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    unsafe {
        let mut settings: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut settings) == 0 {
            settings.c_lflag &= !(libc::ECHO | libc::ECHONL);
            libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &settings);
        }
    }
    Ok((master, slave))
}

/// Reads until steamcmd closes the terminal. Prompts end without a newline, so the unfinished
/// end of the output is checked for one as well, once.
fn listen(terminal: &mut File, heard: &UnboundedSender<Heard>) -> Reading {
    let mut reading = Reading::default();
    let mut pending = String::new();
    let mut prompted = false;
    let mut buffer = [0u8; 4096];
    while let Ok(read) = terminal.read(&mut buffer) {
        if read == 0 {
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&buffer[..read]));
        while let Some(end) = pending.find(['\n', '\r']) {
            let line: String = pending.drain(..=end).collect();
            if let Some(prompt) = reading.line(&line, !prompted) {
                let _ = heard.send(Heard::Asked(prompt));
            }
            prompted = false;
        }
        if !prompted && let Some(prompt) = prompt_in(&pending.trim().to_lowercase()) {
            prompted = true;
            let _ = heard.send(Heard::Asked(prompt));
        }
    }
    reading
}

#[derive(Default)]
struct Reading {
    failure: Option<String>,
    downloaded: bool,
    in_license: bool,
    active: bool,
    owned: bool,
    last: VecDeque<String>,
}

impl Reading {
    /// Takes one whole line of output; a prompt in it is returned unless it was already
    /// answered as the line's unfinished end.
    fn line(&mut self, line: &str, prompts: bool) -> Option<Prompt> {
        let text = line.trim().to_lowercase();
        if text.is_empty() {
            return None;
        }
        self.last.push_back(line.trim().to_owned());
        if self.last.len() > 2 {
            self.last.pop_front();
        }
        if text.starts_with("license packageid") {
            self.in_license = true;
            self.active = false;
        } else if self.in_license && text.starts_with("- state") {
            self.active = text.contains("active");
        } else if self.in_license && text.starts_with("- apps") {
            let apps = text.split_once(':').map_or("", |(_, apps)| apps);
            self.owned |= self.active && apps.split([',', ' ']).any(|app| app == APP);
        }
        if text.contains("success. downloaded item") {
            self.downloaded = true;
        }
        if self.failure.is_none() {
            self.failure = failure(line.trim());
        }
        if prompts { prompt_in(&text) } else { None }
    }

    fn verdict(&self, job: &Job, exited_well: bool, content: &Path) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let said = || self.last.iter().cloned().collect::<Vec<_>>().join(" / ");
        match job {
            Job::SignIn if self.owned => Ok(()),
            Job::SignIn if exited_well => Err(NOT_OWNED.into()),
            Job::Download(id) if self.downloaded && content.join(id).is_dir() => Ok(()),
            _ => Err(format!("steamcmd stopped before it was done: {}", said())),
        }
    }
}

const NOT_OWNED: &str = "This Steam account doesn't own Wallpaper Engine, and the Workshop only gives its items to accounts that do.";

fn prompt_in(text: &str) -> Option<Prompt> {
    if text.ends_with("password:") {
        Some(Prompt::Password)
    } else if text.ends_with("code:") {
        Some(Prompt::Code)
    } else if text.contains("confirm") && (text.contains("mobile") || text.contains("steam guard"))
    {
        Some(Prompt::Approval)
    } else {
        None
    }
}

/// What a failure line means for the person signing in, from the reason steamcmd puts in
/// brackets after `FAILED` or `ERROR`.
fn failure(line: &str) -> Option<String> {
    let text = line.to_ascii_lowercase();
    if text.contains("logged in elsewhere") || text.contains("loggedinelsewhere") {
        return Some(
            "Steam ended the session because the account signed in somewhere else. Try again once that's done."
                .into(),
        );
    }
    let (start, end) = reason(&text)?;
    let reason = line[start..end].trim();
    let key: String = text[start..end]
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let mentions = |words: &[&str]| words.iter().any(|word| key.contains(word));
    Some(
        if mentions(&["invalidpassword", "invalidlogin", "accountnotfound"]) {
            "Steam didn't take that account name and password.".into()
        } else if mentions(&["twofactor", "authcode", "logondenied", "expired"]) {
            "Steam Guard didn't take that code, or it expired. Try again with a fresh one.".into()
        } else if mentions(&["ratelimit"]) {
            "Steam is refusing sign-ins from here for a while. Try again later.".into()
        } else if mentions(&[
            "timeout",
            "noconnection",
            "serviceunavailable",
            "tryanothercm",
        ]) {
            "Steam couldn't be reached. Check the connection and try again.".into()
        } else if mentions(&["accessdenied", "nosubscription", "notowned"]) {
            NOT_OWNED.into()
        } else {
            format!("steamcmd failed: {reason}")
        },
    )
}

/// Where the bracketed reason after `failed` or `error` is in a lowercase line, as in
/// `error (rate limit exceeded)`.
fn reason(text: &str) -> Option<(usize, usize)> {
    ["failed", "error"].iter().find_map(|word| {
        text.match_indices(word).find_map(|(at, _)| {
            let rest = &text[at + word.len()..];
            let inside = rest.trim_start_matches(['!', ' ']).strip_prefix('(')?;
            let start = text.len() - inside.len();
            inside.find(')').map(|end| (start, start + end))
        })
    })
}

fn size_of(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => size_of(&entry.path()),
            _ => entry.metadata().map_or(0, |meta| meta.len()),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(lines: &[&str]) -> (Reading, Vec<Prompt>) {
        let mut reading = Reading::default();
        let prompts = lines
            .iter()
            .filter_map(|line| reading.line(line, true))
            .collect();
        (reading, prompts)
    }

    #[test]
    fn account_names_cannot_smuggle_commands() {
        assert!(valid_account("wall_fan42"));
        assert!(!valid_account("+quit"));
        assert!(!valid_account("me +app_update 1"));
        assert!(!valid_account("ab"));
    }

    #[test]
    fn prompts_and_a_download_are_recognised() {
        let (reading, prompts) = read(&[
            "Logging in user 'wall_fan42' [U:1:0] to Steam Public...",
            "password:",
            "Please check your email for the message from Steam, and enter the Steam Guard ",
            "Steam Guard code:",
            "Please confirm the login in the Steam Mobile app on your phone.",
            "Waiting for user info...OK",
            "Success. Downloaded item 2317494988 to \"/x/2317494988\" (9370203 bytes) ",
        ]);
        assert_eq!(prompts, [Prompt::Password, Prompt::Code, Prompt::Approval]);
        assert!(reading.downloaded && reading.failure.is_none());
    }

    #[test]
    fn failures_say_what_to_do() {
        let says = |line: &str| read(&[line]).0.failure.unwrap_or_default();
        assert!(says("FAILED (Invalid Password)").contains("account name and password"));
        assert!(says("ERROR (Rate Limit Exceeded)").contains("for a while"));
        assert!(says("FAILED (Two-factor code mismatch)").contains("Steam Guard"));
        assert!(says("ERROR! Download item 1 failed (Access Denied).").contains("doesn't own"));
        assert!(says("ERROR! Download item 1 failed (Disk Write Failure).").contains("Disk Write"));
        assert_eq!(
            says("Warning: failed to init SDL thread priority manager: SDL not found"),
            ""
        );
    }

    #[test]
    fn ownership_comes_from_an_active_license_with_the_app() {
        let owned = |lines: &[&str]| read(lines).0.owned;
        assert!(owned(&[
            "License packageid 69373:",
            " - State   : Active( flags 0 ) - Purchased : Sat Oct  1 2022",
            " - Apps    : 431960,  (1 in total)",
        ]));
        assert!(!owned(&[
            "License packageid 69373:",
            " - State   : Expired( flags 4 )",
            " - Apps    : 431960,  (1 in total)",
        ]));
        assert!(!owned(&[
            "License packageid 0:",
            " - State   : Active( flags 0 )",
            " - Apps    : 7, 4319600,  (2 in total)",
        ]));
    }

    /// Runs the stand-in from tests/steamcmd, answering its questions from `answers`.
    async fn stand_in(
        job: Job,
        account: &str,
        root: &Path,
        answers: &[&str],
    ) -> (Vec<Heard>, Result<(), String>) {
        let setup = Setup {
            program: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/steamcmd/stand-in.sh")
                .into(),
            root: root.to_path_buf(),
        };
        let (input, inputs) = tokio::sync::mpsc::unbounded_channel();
        let mut heard = start_with(setup, job, account.into(), inputs);
        let mut answers = answers.iter();
        let mut seen = Vec::new();
        while let Some(news) = heard.recv().await {
            match news {
                Heard::Done(result) => return (seen, result),
                Heard::Asked(prompt) => {
                    if let Some(answer) = answers.next() {
                        input.send(Input::Answer((*answer).into())).unwrap();
                    }
                    seen.push(Heard::Asked(prompt));
                }
                Heard::Progress(_) => {}
            }
        }
        (seen, Err("no verdict".into()))
    }

    #[tokio::test]
    async fn signs_in_through_its_prompts_then_downloads_on_the_kept_sign_in() {
        let root = std::env::temp_dir().join(format!("apolline-steamcmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (asked, result) = stand_in(
            Job::SignIn,
            "stand_in",
            &root,
            &["stand-in-password", "QX7RM"],
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            asked,
            [Heard::Asked(Prompt::Password), Heard::Asked(Prompt::Code)]
        );
        let (asked, result) = stand_in(Job::Download("42".into()), "stand_in", &root, &[]).await;
        assert_eq!((asked, result), (vec![], Ok(())));
        let item = root.join("steamapps/workshop/content/431960/42");
        assert!(item.join("project.json").is_file());
        assert!(!root.join("steamapps/workshop/downloads/431960/42").exists());
        let (_, result) = stand_in(Job::Download("13".into()), "stand_in", &root, &[]).await;
        assert!(result.unwrap_err().contains("doesn't own"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn a_wrong_password_says_so_and_a_bad_name_never_runs() {
        let root =
            std::env::temp_dir().join(format!("apolline-steamcmd-no-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (_, result) = stand_in(Job::SignIn, "stand_in", &root, &["guess"]).await;
        assert!(result.unwrap_err().contains("account name and password"));
        let (asked, result) = stand_in(Job::SignIn, "+quit", &root, &[]).await;
        assert!(asked.is_empty() && result.unwrap_err().contains("isn't a Steam account"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn update_times_come_from_the_manifest() {
        let acf = r#""AppWorkshop"
{
	"appid"		"431960"
	"WorkshopItemsInstalled"
	{
		"2317494988"
		{
			"size"		"9370203"
			"timeupdated"		"1793296108"
			"manifest"		"123"
		}
	}
	"WorkshopItemDetails"
	{
		"2317494988"
		{
			"manifest"		"123"
			"timeupdated"		"1"
		}
	}
}"#;
        assert_eq!(times_in(acf).get("2317494988"), Some(&1_793_296_108));
    }
}
