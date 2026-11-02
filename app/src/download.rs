use std::collections::{HashSet, VecDeque};

use iced::Task;
use tokio::sync::mpsc::UnboundedSender;

use crate::steamcmd::{self, Heard, Input, Job, Prompt};
use crate::{App, Message, config, workshop};

/// steamcmd's side of the app: the account, the job running now and the ones waiting.
#[derive(Default)]
pub struct Steam {
    /// The account name as typed in Settings.
    pub account: String,
    pub run: Option<Run>,
    pub queue: VecDeque<Wanted>,
    /// What is being typed for steamcmd's question, sent and then forgotten.
    pub answer: String,
    /// Signed in during this run of the app, on the account as typed.
    pub signed_in: bool,
}

pub struct Run {
    pub job: Job,
    pub title: String,
    pub size: u64,
    pub asked: Option<Prompt>,
    pub done: u64,
    input: UnboundedSender<Input>,
}

#[derive(Clone, Debug)]
pub struct Wanted {
    pub id: String,
    pub title: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub enum Act {
    Account(String),
    SignIn,
    Download(Wanted),
    Heard(Heard),
    Answer(String),
    Send,
    Cancel,
    Updates(Result<HashSet<String>, String>),
}

impl Steam {
    pub fn new(account: Option<&str>) -> Self {
        Steam {
            account: account.unwrap_or_default().to_owned(),
            ..Steam::default()
        }
    }

    /// Whether this item is downloading now or waiting to.
    pub fn wants(&self, id: &str) -> bool {
        self.queue.iter().any(|wanted| wanted.id == id)
            || self
                .run
                .as_ref()
                .is_some_and(|run| run.job == Job::Download(id.to_owned()))
    }
}

impl App {
    pub fn steam(&mut self, act: Act) -> Task<Message> {
        match act {
            Act::Account(name) => {
                self.steam.account = name;
                self.steam.signed_in = false;
                Task::none()
            }
            Act::SignIn => {
                let name = self.steam.account.trim().to_owned();
                if !steamcmd::valid_account(&name) {
                    self.notice = Some(
                        "A Steam account name is 3 to 64 letters, digits or underscores.".into(),
                    );
                    return Task::none();
                }
                self.config.steam_account = Some(name);
                self.notice = config::save(&self.config).err();
                self.begin(Job::SignIn, "Signing in".into(), 0)
            }
            Act::Download(wanted) => {
                if self.steam.wants(&wanted.id) {
                    return Task::none();
                }
                self.steam.queue.push_back(wanted);
                self.next()
            }
            Act::Heard(Heard::Asked(prompt)) => {
                if let Some(run) = &mut self.steam.run {
                    run.asked = Some(prompt);
                }
                Task::none()
            }
            Act::Heard(Heard::Progress(bytes)) => {
                if let Some(run) = &mut self.steam.run {
                    run.done = bytes;
                    // A download moving along means any sign-in question was answered.
                    if bytes > 0 {
                        run.asked = None;
                    }
                }
                Task::none()
            }
            Act::Heard(Heard::Done(result)) => self.finished(result),
            Act::Answer(text) => {
                self.steam.answer = text;
                Task::none()
            }
            Act::Send => {
                let answer = std::mem::take(&mut self.steam.answer);
                if let Some(run) = &mut self.steam.run
                    && !answer.is_empty()
                {
                    run.asked = None;
                    let _ = run.input.send(Input::Answer(answer));
                }
                Task::none()
            }
            Act::Cancel => {
                self.steam.queue.clear();
                if let Some(run) = &self.steam.run {
                    let _ = run.input.send(Input::Cancel);
                }
                Task::none()
            }
            Act::Updates(Ok(updates)) => {
                self.browser.updates = updates;
                Task::none()
            }
            Act::Updates(Err(_)) => Task::none(),
        }
    }

    /// Asks the Workshop about the items steamcmd downloaded here.
    pub fn check_updates(&self) -> Task<Message> {
        let local = steamcmd::versions();
        if local.is_empty() {
            return Task::none();
        }
        Task::perform(workshop::updates(local), |found| {
            Message::Steam(Act::Updates(found))
        })
    }

    fn next(&mut self) -> Task<Message> {
        if self.steam.run.is_some() {
            return Task::none();
        }
        match self.steam.queue.pop_front() {
            Some(wanted) => self.begin(Job::Download(wanted.id), wanted.title, wanted.size),
            None => Task::none(),
        }
    }

    fn begin(&mut self, job: Job, title: String, size: u64) -> Task<Message> {
        let Some(account) = self.config.steam_account.clone() else {
            self.steam.queue.clear();
            self.notice = Some("Add your Steam account in Settings first.".into());
            return Task::none();
        };
        self.notice = None;
        self.steam.answer.clear();
        let (input, inputs) = tokio::sync::mpsc::unbounded_channel();
        let heard = steamcmd::start(job.clone(), account, inputs);
        self.steam.run = Some(Run {
            job,
            title,
            size,
            asked: None,
            done: 0,
            input,
        });
        let stream = iced::futures::stream::unfold(heard, |mut heard| async move {
            heard.recv().await.map(|news| (news, heard))
        });
        Task::run(stream, |news| Message::Steam(Act::Heard(news)))
    }

    fn finished(&mut self, result: Result<(), String>) -> Task<Message> {
        let Some(run) = self.steam.run.take() else {
            return Task::none();
        };
        self.steam.answer.clear();
        let after = match (&run.job, result) {
            (Job::SignIn, Ok(())) => {
                self.steam.signed_in = true;
                Task::none()
            }
            (Job::Download(id), Ok(())) => {
                self.browser.updates.remove(id);
                self.rescan()
            }
            (_, Err(message)) => {
                // What stopped one job, a sign-in or the connection, would stop the rest.
                self.steam.queue.clear();
                self.notice = Some(message);
                Task::none()
            }
        };
        Task::batch([after, self.next()])
    }
}
