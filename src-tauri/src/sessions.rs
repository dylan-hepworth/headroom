// The Claude Code sessions Headroom knows about, built from the log the hooks write (see hooks.rs).
//
// Each line in the log is one event from one session. We read new lines as they arrive and keep where every session
// stands: working, waiting on permission, waiting for the user's next message, stopped at a usage limit, or idle.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use chrono::{DateTime, Local, TimeZone};
use serde_json::{json, Value};

/// Once the log gets this big and we've read all of it, we empty it so it doesn't grow forever.
const MAX_LOG: u64 = 2_000_000;

/// A session at work that we haven't heard from in this long is left out of the list: it was likely interrupted, which
/// Claude Code doesn't tell its hooks about.
const QUIET_WORK: chrono::Duration = chrono::Duration::minutes(30);

/// Sessions we haven't heard from in this long are dropped. A session that ended without telling us (its terminal was
/// closed, or the Mac went to sleep) would otherwise sit in the list forever.
const FORGET_AFTER: chrono::Duration = chrono::Duration::hours(12);

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    /// Asking permission to use a tool.
    Permission,
    /// Asking the user a question.
    Question,
    /// Done with its turn, waiting for the user's next message.
    Waiting,
    Working,
    /// Stopped because it hit a usage limit.
    Limited,
    /// Started, but nothing's been asked of it yet.
    Idle,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Permission => "permission",
            Status::Question => "question",
            Status::Waiting => "waiting",
            Status::Working => "working",
            Status::Limited => "limited",
            Status::Idle => "idle",
        }
    }
}

pub struct Session {
    cwd: String,
    /// The chat's title, as the Claude app shows it, once we know it.
    title: Option<String>,
    /// The bundle ID of the app the session runs in (a terminal, or the Claude app), when the hook could tell.
    app: Option<String>,
    /// In the Claude app, the app's own ID for its chat ("local_…"), from the hooks.
    host_chat: Option<String>,
    /// When the user last sent a message, so we know how long Claude has been at it.
    turn_started: Option<DateTime<Local>>,
    status: Status,
    /// When it got to its current status.
    since: DateTime<Local>,
    /// When we last heard from it at all.
    heard: DateTime<Local>,
    /// What it's doing while it works, e.g. "Ran npm test".
    activity: Option<String>,
    /// What it's asking permission for: the tool, and its command or file.
    request: Option<(String, String)>,
    /// The start of Claude's last reply, once it's done.
    last: Option<String>,
    /// For a session in the Claude app: how the app sorted its last turn (see desktop.rs).
    chat: Option<crate::desktop::Chat>,
    /// Its finished turn has been seen: the chat was open in the Claude app when it ended, or has been opened since.
    seen: bool,
    /// Run by a script or an app built on the Agent SDK, with nobody there to answer it.
    unattended: bool,
    /// The process that last ran one of its hooks: its Claude Code, or a shell in between.
    hook_parent: Option<i32>,
}

/// A permission request or question that a hook is holding for the user to answer in Headroom. These are kept apart
/// from the session's status: a session can run other tools while one waits, or ask again before we hear the first
/// was answered, and neither ends the wait.
#[derive(Clone)]
struct Held {
    /// The session asking.
    session: String,
    /// When the hook started holding it.
    since: DateTime<Local>,
    /// How long the hook holds it for, before the time the user adds by pausing.
    hold: Option<chrono::Duration>,
    /// The hook's process. If it's gone, so is the request, whatever the log says.
    pid: Option<i32>,
    tool: String,
    /// The tool's whole command or file.
    detail: String,
    /// The same, clipped the way every log line clips it, to match it with the tool call once it runs.
    short: String,
    /// Did Claude Code suggest a rule, so that "Allow for Session" can apply?
    can_session: bool,
    /// Hands-free: not a request but a finished turn, held open a while for a reply from the popover.
    reply: bool,
    /// For questions: each question, its options, and whether more than one can be picked.
    questions: Option<Value>,
    /// Hands-free: what the user last said, and what Claude has said since (see hooks.rs `save_context`).
    context: Option<Value>,
}

/// Something that happened to a session that the user might want a notification about.
pub struct Change {
    pub at: DateTime<Local>,
    /// The session's ID, so clicking the notification can open it.
    pub session: String,
    pub project: String,
    pub app: Option<String>,
    pub kind: ChangeKind,
}

pub enum ChangeKind {
    /// It asked permission for `tool` (with its command or file), or asked the user a question. `request` is set when a
    /// hook is holding it for the popover, and `late` when it's just gone back to the terminal unanswered.
    NeedsAnswer { question: bool, tool: String, detail: String, request: Option<String>, late: bool },
    /// It finished its turn, after working for `ran`.
    Finished { reply: Option<String>, ran: Option<chrono::Duration> },
}

#[derive(Default)]
pub struct Sessions {
    sessions: HashMap<String, Session>,
    /// Requests held for an answer, by request ID.
    held: HashMap<String, Held>,
    /// Held requests that were settled some other way (see `release`), whose hooks are still waiting and should be told
    /// to stop.
    released: Vec<String>,
    /// How far into the log we've read.
    offset: u64,
}

impl Sessions {
    /// Read whatever's been added to the log since last time. Returns whether anything changed, and the changes the
    /// user might want to hear about.
    pub fn read(&mut self, path: &Path) -> (bool, Vec<Change>) {
        let mut changes = vec![];
        let Ok(mut file) = File::open(path) else { return (false, changes) };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            // Emptied since we last looked
            self.offset = 0;
        }
        if len == self.offset {
            return (false, changes);
        }
        let mut text = String::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_string(&mut text).is_err() {
            return (false, changes);
        }

        // A hook could be partway through writing the last line, so we only take up to the last full one
        let Some(end) = text.rfind('\n') else { return (false, changes) };
        for line in text[..=end].lines() {
            if let Ok(event) = serde_json::from_str::<Value>(line) {
                changes.extend(self.apply(&event));
            }
        }
        self.offset += end as u64 + 1;

        if self.offset >= MAX_LOG && self.offset == len {
            let _ = File::create(path);
            self.offset = 0;
        }
        (true, changes)
    }

    /// Update a session with one event from its hooks, and return the change it made, if it's one worth telling the
    /// user about.
    fn apply(&mut self, event: &Value) -> Option<Change> {
        let id = event["session_id"].as_str()?;
        let name = event["hook_event_name"].as_str().unwrap_or_default();
        let at = event["at"].as_i64().and_then(|ms| Local.timestamp_millis_opt(ms).single()).unwrap_or_else(Local::now);
        let request = event["request_id"].as_str();
        let tool = event["tool_name"].as_str().unwrap_or_default();
        let detail = event["detail"].as_str().unwrap_or_default();
        if name == "SessionEnd" {
            self.sessions.remove(id);
            self.release(|h| h.session == id);
            return None;
        }
        // Sent back to ask what's next, or given a message the user sent from the list (see hooks.rs), the turn goes on
        let nudged = name == "Stop" && event["nudged"] == true;
        let interjected = event["interjected"] == true;
        let goes_on = nudged || (name == "Stop" && interjected);
        // The turn ended, or the user moved on, so nothing it asked is still waiting
        if matches!(name, "UserPromptSubmit" | "Stop" | "StopFailure") && !goes_on {
            self.release(|h| h.session == id);
        }
        // The tool it asked about ran: Claude Code's own prompt was answered in the terminal
        if matches!(name, "PostToolUse" | "PostToolUseFailure") {
            self.release(|h| h.session == id && h.questions.is_none() && h.tool == tool && h.short == detail);
        }
        // The end of a request that may belong to a session that has ended since. That's no reason to bring it back.
        if matches!(name, "HeadroomAnswered" | "PermissionFallback" | "HeadroomInterjected" | "HeadroomSeen")
            && !self.sessions.contains_key(id)
        {
            self.held.remove(request?);
            return None;
        }

        let session = self.sessions.entry(id.to_string()).or_insert_with(|| Session {
            cwd: String::new(),
            title: None,
            app: None,
            host_chat: None,
            turn_started: None,
            status: Status::Idle,
            since: at,
            heard: at,
            activity: None,
            request: None,
            last: None,
            chat: None,
            seen: false,
            unattended: false,
            hook_parent: None,
        });
        if let Some(cwd) = event["cwd"].as_str() {
            session.cwd = cwd.to_string();
        }
        if let Some(title) = event["title"].as_str() {
            session.title = Some(title.to_string());
        }
        if let Some(app) = event["app"].as_str() {
            session.app = Some(app.to_string());
        }
        if let Some(chat) = event["chat"].as_str() {
            session.host_chat = Some(chat.to_string());
        }
        // Marked as seen from the list (the app logs this, so a restart doesn't bring it back). Not word from the
        // session itself.
        if name == "HeadroomSeen" {
            session.seen |= session.status == Status::Waiting;
            return None;
        }
        session.heard = at;
        session.unattended |= event["unattended"] == true;
        if let Some(pid) = event["ppid"].as_i64().and_then(|p| i32::try_from(p).ok()) {
            session.hook_parent = Some(pid);
        }

        let status = match name {
            "SessionStart" => Status::Idle,
            "UserPromptSubmit" => {
                session.activity = Some("Thinking".into());
                session.turn_started = Some(at);
                Status::Working
            }
            "PermissionRequest" | "PreToolUse" if name == "PermissionRequest" || tool == "AskUserQuestion" => {
                if let Some(request) = request {
                    self.held.insert(
                        request.to_string(),
                        Held {
                            session: id.to_string(),
                            since: at,
                            hold: event["hold"].as_i64().map(chrono::Duration::seconds),
                            pid: event["pid"].as_i64().and_then(|p| i32::try_from(p).ok()),
                            tool: tool.to_string(),
                            detail: event["full_detail"].as_str().unwrap_or(detail).to_string(),
                            short: detail.to_string(),
                            can_session: event["can_session"].as_bool().unwrap_or(false),
                            // A log from before several questions could be held has just the one
                            questions: event
                                .get("questions")
                                .cloned()
                                .or_else(|| event.get("question").map(|q| json!([q]))),
                            context: (event["context"] == true).then(|| crate::hooks::context(request)).flatten(),
                            reply: false,
                        },
                    );
                }
                // A question comes through just before Claude asks it (and sometimes as a permission request too)
                if tool == "AskUserQuestion" {
                    session.request = Some(("Question".into(), detail.to_string()));
                    Status::Question
                } else {
                    session.request = Some((tool.to_string(), detail.to_string()));
                    Status::Permission
                }
            }
            // The user answered a held request in Headroom (the app logs this, so a restart doesn't bring it back)
            "HeadroomAnswered" => {
                let was_held = request.and_then(|r| self.held.remove(r)).is_some();
                let still_asking = self.held.values().any(|h| h.session == id);
                if was_held && !still_asking {
                    session.request = None;
                    session.activity = Some("Thinking".into());
                    Status::Working
                } else {
                    session.status
                }
            }
            // The hook stopped holding the request, and Claude Code is asking in the terminal now. If it was still held
            // here when its time was up, nobody answered or handed it back.
            "PermissionFallback" => {
                let held = request.and_then(|r| self.held.remove(r));
                // Not one that ended early, with the session interrupted, say
                let ran_out = |h: &Held| h.hold.is_none_or(|hold| at >= h.since + hold - chrono::Duration::seconds(2));
                // A turn held open for a reply that wasn't sent is just a finished turn: nothing to say
                if let Some(held) = held.filter(ran_out).filter(|h| !h.reply) {
                    let (tool, detail) = match &held.questions {
                        Some(q) => ("Question".to_string(), q[0]["text"].as_str().unwrap_or_default().to_string()),
                        None => (held.tool, held.short),
                    };
                    let question = held.questions.is_some();
                    let kind = ChangeKind::NeedsAnswer { question, tool, detail, request: None, late: true };
                    return Some(Change {
                        at,
                        session: id.to_string(),
                        project: session.name(),
                        app: session.app.clone(),
                        kind,
                    });
                }
                session.status
            }
            // A message the user sent from the list reached Claude: after a tool call, at the end of the turn, or as
            // the reply to a turn held open for one, which isn't waiting for a reply any more
            _ if interjected || name == "HeadroomInterjected" => {
                if session.status == Status::Waiting {
                    session.turn_started = Some(at);
                }
                self.held.retain(|_, h| !(h.session == id && h.reply));
                session.activity = Some("Reading your message".into());
                Status::Working
            }
            "PostToolUse" | "PostToolUseFailure" => {
                session.activity = Some(activity(tool, detail));
                Status::Working
            }
            "Stop" if nudged => {
                session.activity = Some("Asking what's next".into());
                Status::Working
            }
            "Stop" => {
                // Hands-free, the hook holds the finished turn open a while for a reply from the popover
                if let Some(request) = request.filter(|_| event["reply"] == true) {
                    self.held.insert(
                        request.to_string(),
                        Held {
                            session: id.to_string(),
                            since: at,
                            hold: event["hold"].as_i64().map(chrono::Duration::seconds),
                            pid: event["pid"].as_i64().and_then(|p| i32::try_from(p).ok()),
                            tool: "Reply".into(),
                            detail: String::new(),
                            short: String::new(),
                            can_session: false,
                            questions: None,
                            context: (event["context"] == true).then(|| crate::hooks::context(request)).flatten(),
                            reply: true,
                        },
                    );
                }
                session.last = event["last_assistant_message"].as_str().map(String::from);
                session.seen = false;
                session.chat = None;
                Status::Waiting
            }
            "StopFailure" if event["error"].as_str() == Some("rate_limit") => Status::Limited,
            "StopFailure" => Status::Waiting,
            _ => session.status,
        };
        if !matches!(status, Status::Permission | Status::Question) {
            session.request = None;
        }
        if status == session.status {
            return None;
        }
        session.status = status;
        session.since = at;

        let kind = match status {
            Status::Permission | Status::Question => {
                let (tool, detail) = session.request.clone().unwrap_or_default();
                let request = request.map(String::from);
                ChangeKind::NeedsAnswer { question: status == Status::Question, tool, detail, request, late: false }
            }
            Status::Waiting => ChangeKind::Finished {
                reply: session.last.clone(),
                ran: session.turn_started.take().map(|started| at - started),
            },
            _ => return None,
        };
        Some(Change { at, session: id.to_string(), project: session.name(), app: session.app.clone(), kind })
    }

    /// Stop holding the requests that match, because they were settled some other way. The hooks still holding them
    /// are waiting for an answer that won't come, so they're noted for the app to send each one back to the terminal
    /// (see `take_released`). Only hooks still running: reading an old log again after a restart shouldn't leave
    /// answers behind for hooks long gone.
    fn release(&mut self, matches: impl Fn(&Held) -> bool) {
        let released = &mut self.released;
        self.held.retain(|id, h| {
            if !matches(h) {
                return true;
            }
            if h.pid.is_some_and(running) {
                released.push(id.clone());
            }
            false
        });
    }

    /// The held requests released since last time, whose hooks should be told to stop waiting.
    pub fn take_released(&mut self) -> Vec<String> {
        std::mem::take(&mut self.released)
    }

    /// The session a held request belongs to.
    pub fn session_of(&self, request: &str) -> Option<String> {
        self.held.get(request).map(|h| h.session.clone())
    }

    /// Whether a held request is a question ("question"), a permission request ("permission"), or a finished turn held
    /// open for a reply ("reply").
    pub fn kind_of(&self, request: &str) -> Option<&'static str> {
        self.held.get(request).map(|h| match (h.reply, &h.questions) {
            (true, _) => "reply",
            (false, Some(_)) => "question",
            (false, None) => "permission",
        })
    }

    /// The user answered a held request in Headroom. The session gets back to work right away, rather than showing
    /// "needs permission" until the tool finishes.
    pub fn answered(&mut self, request: &str) {
        let Some(held) = self.held.remove(request) else { return };
        if self.held.values().any(|h| h.session == held.session) {
            return;
        }
        if let Some(session) = self.sessions.get_mut(&held.session) {
            session.request = None;
            session.status = Status::Working;
            session.since = Local::now();
            session.activity = Some("Thinking".into());
        }
    }

    /// The user handed a held request back to Claude Code's own prompt. It's still waiting on them, but not here.
    pub fn handed_back(&mut self, request: &str) {
        self.held.remove(request);
    }

    /// The held requests still waiting, with when each runs out. `hold` is how long a request is held when its log line
    /// doesn't say (one from an older Headroom), and `extra` any time the user added by pausing. A request past its
    /// time, or whose hook isn't running any more, was given up on, even if we never heard.
    fn live<'a>(
        &'a self,
        hold: chrono::Duration,
        extra: &'a HashMap<String, i64>,
    ) -> impl Iterator<Item = (&'a String, &'a Held, DateTime<Local>)> {
        let now = Local::now();
        self.held.iter().filter_map(move |(id, held)| {
            let hold = held.hold.unwrap_or(hold);
            let until = held.since + hold + chrono::Duration::milliseconds(*extra.get(id).unwrap_or(&0));
            (until > now && held.pid.is_none_or(running) && self.sessions.contains_key(&held.session))
                .then_some((id, held, until))
        })
    }

    /// The requests Headroom is holding for an answer, for the popover, oldest first. Not the finished turns held open
    /// for a reply: those wait quietly, for the list of what's pending (see `pending`).
    pub fn held(&self, hold: chrono::Duration, extra: &HashMap<String, i64>) -> Vec<Value> {
        let mut list: Vec<(DateTime<Local>, Value)> = self
            .live(hold, extra)
            .filter(|(_, held, _)| !held.reply)
            .map(|(id, held, until)| {
                let session = &self.sessions[&held.session];
                let project = project_name(&session.cwd);
                let mut ask = json!({
                    "id": id,
                    "session": held.session,
                    "title": session.title.clone().unwrap_or_else(|| project.clone()),
                    "project": project,
                    "until": until.timestamp_millis(),
                    "hold": held.hold.unwrap_or(hold).num_milliseconds(),
                    "inChat": session.host_chat.is_some(),
                });
                match &held.questions {
                    Some(questions) => {
                        ask["kind"] = "question".into();
                        // In the shape the popover takes: the log keeps each question's words as "text"
                        let shown: Vec<Value> = questions
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|q| json!({ "question": q["text"], "options": q["options"], "multiple": q["multiple"] }))
                            .collect();
                        ask["questions"] = shown.into();
                        if let Some(context) = &held.context {
                            ask["said"] = context["said"].clone();
                            ask["prompt"] = context["prompt"].clone();
                            ask["recent"] = context["recent"].clone();
                        }
                    }
                    None => {
                        ask["kind"] = "permission".into();
                        ask["tool"] = held.tool.clone().into();
                        ask["detail"] = held.detail.clone().into();
                        ask["canSession"] = held.can_session.into();
                    }
                }
                (held.since, ask)
            })
            .collect();
        list.sort_by_key(|(since, _)| *since);
        list.into_iter().map(|(_, ask)| ask).collect()
    }

    /// What a session's icon is kept under (see icons.rs): its chat in the Claude app, or itself, and its folder.
    pub fn icon_key(&self, id: &str) -> Option<(String, String)> {
        let s = self.sessions.get(id)?;
        Some((s.host_chat.clone().unwrap_or_else(|| id.to_string()), s.cwd.clone()))
    }

    /// The icon keys of the sessions in a folder.
    pub fn chats_in(&self, cwd: &str) -> Vec<String> {
        self.sessions.keys().filter_map(|id| self.icon_key(id).filter(|(_, c)| c == cwd).map(|(key, _)| key)).collect()
    }

    /// The process that last ran one of a session's hooks (see hooks.rs `keep`).
    pub fn hook_parent(&self, id: &str) -> Option<i32> {
        self.sessions.get(id)?.hook_parent
    }

    /// The bundle ID of the app a session runs in, when the hook could tell.
    pub fn app_of(&self, id: &str) -> Option<String> {
        self.sessions.get(id)?.app.clone()
    }

    /// The Claude app's own ID for a session's chat, when the hooks said.
    pub fn host_chat(&self, id: &str) -> Option<String> {
        self.sessions.get(id)?.host_chat.clone()
    }

    /// Every session's ID and what to call it.
    pub fn names(&self) -> Vec<(String, String)> {
        self.sessions.iter().map(|(id, s)| (id.clone(), s.name())).collect()
    }

    /// The names of the sessions that stopped because they hit a usage limit.
    pub fn limited(&self) -> Vec<String> {
        let mut projects: Vec<String> =
            self.sessions.values().filter(|s| s.status == Status::Limited).map(Session::name).collect();
        projects.sort();
        projects.dedup();
        projects
    }

    /// Drop sessions we haven't heard from in a long time. Returns true if any were dropped.
    pub fn forget_quiet(&mut self) -> bool {
        let before = self.sessions.len();
        let cutoff = Local::now() - FORGET_AFTER;
        self.sessions.retain(|_, s| s.heard > cutoff);
        let sessions = &self.sessions;
        self.held.retain(|_, h| sessions.contains_key(&h.session));
        self.sessions.len() != before
    }

    /// How many sessions need an answer from the user, and how many are done and waiting for their next message. The
    /// menu bar shows these as yellow and blue dots, the way the Claude app's list of chats does: yellow for a
    /// permission request or a question, and for a turn Claude ended by asking something in plain text; blue for a
    /// finished turn. A finished turn stops counting once it's been seen in the app, and archived chats don't count.
    pub fn waiting(&self) -> (usize, usize) {
        let (mut answer, mut done) = (0, 0);
        for s in self.sessions.values() {
            let chat = s.chat.unwrap_or_default();
            match s.status {
                Status::Permission | Status::Question => answer += 1,
                Status::Waiting if s.seen || chat.archived => {}
                Status::Waiting if chat.needs_you => answer += 1,
                Status::Waiting => done += 1,
                _ => {}
            }
        }
        (answer, done)
    }

    /// Hands-free: the sessions for the list that drops down from the menu bar item, most recent first. The ones the
    /// dots count, asking something (with the request Headroom's holding if it is) or done and not seen yet (with the
    /// turn held open for a reply if it is), and the ones at work, which a message can be sent to. `hold` and `extra`
    /// are as for `held`.
    pub fn pending(&self, hold: chrono::Duration, extra: &HashMap<String, i64>) -> Vec<Value> {
        let now = Local::now();
        let live: Vec<(&String, &Held, DateTime<Local>)> = self.live(hold, extra).collect();
        let held_by = |id: &str, reply: bool| {
            live.iter().find(|(_, h, _)| h.session == id && h.reply == reply).map(|(request, ..)| request.to_string())
        };
        let mut list: Vec<(&DateTime<Local>, Value)> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                let chat = s.chat.unwrap_or_default();
                match s.status {
                    Status::Permission | Status::Question => true,
                    Status::Waiting => !s.seen && !chat.archived,
                    Status::Working => !s.unattended && now - s.heard < QUIET_WORK,
                    _ => false,
                }
            })
            .map(|(id, s)| {
                let project = project_name(&s.cwd);
                let asking = matches!(s.status, Status::Permission | Status::Question);
                let state = match s.status {
                    Status::Permission => "permission",
                    Status::Question => "question",
                    Status::Working => "working",
                    _ if s.chat.unwrap_or_default().needs_you => "question",
                    _ => "done",
                };
                let what = match s.status {
                    _ if asking => s.request.as_ref().map(|(_, detail)| detail.clone()),
                    Status::Working => s.activity.clone(),
                    _ => s.last.clone(),
                };
                // At work, since the user's message
                let since = if s.status == Status::Working { s.turn_started.unwrap_or(s.since) } else { s.since };
                let row = json!({
                    "id": id,
                    "title": s.title.clone().unwrap_or_else(|| project.clone()),
                    "project": project,
                    "state": state,
                    "since": since.timestamp_millis(),
                    "what": what.unwrap_or_default(),
                    "heldId": held_by(id, false),
                    "replyId": held_by(id, true),
                    "inChat": s.host_chat.is_some(),
                });
                (&s.since, row)
            })
            .collect();
        list.sort_by(|a, b| b.0.cmp(a.0));
        list.into_iter().map(|(_, row)| row).collect()
    }

    /// The sessions that have finished a turn, for checking with the Claude app.
    pub fn finished(&self) -> Vec<String> {
        self.sessions.iter().filter(|(_, s)| s.status == Status::Waiting).map(|(id, _)| id.clone()).collect()
    }

    /// The user's seen a finished session's turn, from the list: it stops counting, as if its chat had been opened.
    /// Until its next turn ends. Returns whether that changes its dot.
    pub fn mark_seen(&mut self, id: &str) -> bool {
        let Some(s) = self.sessions.get_mut(id).filter(|s| s.status == Status::Waiting) else { return false };
        !std::mem::replace(&mut s.seen, true)
    }

    /// What the Claude app says about a finished session: how it sorted the turn, and whether its chat is open now.
    /// Returns whether that changes its dot.
    pub fn from_app(&mut self, id: &str, chat: Option<crate::desktop::Chat>, open: bool) -> bool {
        let Some(s) = self.sessions.get_mut(id).filter(|s| s.status == Status::Waiting) else { return false };
        let before = (s.chat, s.seen);
        s.chat = chat;
        s.seen |= open;
        (s.chat, s.seen) != before
    }

    /// The sessions for the settings window, most recently active first. `hold` and `extra` are as for `held`, so that
    /// only requests still waiting get buttons to answer them, and `answerable` is false while approvals are off, when
    /// there's nothing to answer here.
    pub fn to_json(&self, hold: chrono::Duration, extra: &HashMap<String, i64>, answerable: bool) -> Value {
        // Each session's oldest request still held, which is the one the popover shows first
        let mut held: HashMap<&str, (&String, &Held)> = HashMap::new();
        // Not a finished turn held open for a reply: that's for the list, and it's no request to allow or deny
        for (id, h, _) in self.live(hold, extra).filter(|(_, h, _)| answerable && !h.reply) {
            let entry = held.entry(h.session.as_str()).or_insert((id, h));
            if h.since < entry.1.since {
                *entry = (id, h);
            }
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let mut list: Vec<_> = self.sessions.iter().collect();
        list.sort_by(|a, b| b.1.heard.cmp(&a.1.heard));
        let list: Vec<Value> = list
            .into_iter()
            .map(|(id, s)| {
                let project = project_name(&s.cwd);
                let title = s.title.clone();
                let path = match s.cwd.strip_prefix(&home) {
                    Some(rest) if !home.is_empty() => format!("~{rest}"),
                    _ => s.cwd.clone(),
                };
                json!({
                    "id": id,
                    "project": project,
                    "title": title,
                    "path": path,
                    "state": s.status.name(),
                    "since": s.since.timestamp_millis(),
                    "activity": if s.status == Status::Working { s.activity.clone() } else { None },
                    "request": s.request.as_ref().map(|(tool, detail)| json!({ "tool": tool, "detail": detail })),
                    // The held request the buttons answer, which isn't always the one in "request"
                    "held": held.get(id.as_str()).map(|(request, h)| json!({
                        "id": request,
                        "canSession": h.can_session,
                        "kind": if h.questions.is_some() { "question" } else { "permission" },
                        "tool": h.tool,
                        "detail": match &h.questions {
                            Some(q) => q[0]["text"].as_str().unwrap_or_default().to_string(),
                            None => h.detail.clone(),
                        },
                    })),
                    "last": if s.status == Status::Waiting { s.last.clone() } else { None },
                })
            })
            .collect();
        Value::Array(list)
    }
}

impl Session {
    /// What to call the session: its chat title, or the folder it's in when it doesn't have one.
    fn name(&self) -> String {
        self.title.clone().unwrap_or_else(|| project_name(&self.cwd))
    }
}

/// Is this process still running? Signal 0 checks without sending anything. A process we're not allowed to signal is
/// still running; only "no such process" means it's gone.
fn running(pid: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const ESRCH: i32 = 3;
    // SAFETY: kill with signal 0 only checks that the process exists and sends nothing
    let signalled = unsafe { kill(pid, 0) } == 0;
    signalled || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

/// A project's name as we show it: the last part of the folder it's in.
fn project_name(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or(cwd.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The part of a shell command worth showing on one line: its first real step, without the setup it's often run with
/// (`cd somewhere &&`, `source ~/.nvm/nvm.sh;`, `export X=1;`, or `X=1` in front of it), a program run by its path
/// named on its own, a script written into the command named for its language, and no longer than about 60 characters.
fn short_command(command: &str) -> String {
    let steps = steps(command);
    let setup = |step: &str| {
        let word = step.split_whitespace().next().unwrap_or_default();
        matches!(word, "cd" | "pushd" | "popd" | "source" | "." | "export" | "set" | "unset" | "nvm" | "trap")
            || step.split_whitespace().all(assignment)
    };
    let step = steps.iter().map(String::as_str).find(|step| !setup(step)).or(steps.last().map(String::as_str));
    let step = step.unwrap_or_default();
    // Without the variables set in front of it
    let words: Vec<&str> = step.split_whitespace().collect();
    let skip = words.iter().take_while(|w| assignment(w)).count();
    let step = words.get(skip).and_then(|first| step.find(first)).map_or(step, |at| &step[at..]);
    let (program, rest) = step.split_once(char::is_whitespace).unwrap_or((step, ""));
    let program = program.rsplit('/').next().unwrap_or(program);
    // A script written into the command, which says nothing on its first line: "python3 - <<'EOF'"
    let inline = rest.contains("<<") || (step.contains('\n') && [" -c ", " -e "].iter().any(|f| step.contains(f)));
    let language = match program {
        "python" | "python3" => Some("Python"),
        "node" | "bun" | "deno" => Some("JavaScript"),
        "ruby" => Some("Ruby"),
        "perl" => Some("Perl"),
        "bash" | "sh" | "zsh" => Some("shell"),
        "osascript" => Some("AppleScript"),
        _ => None,
    };
    if let (true, Some(language)) = (inline, language) {
        return format!("a {language} script");
    }
    let line = format!("{program} {}", rest.lines().next().unwrap_or_default().trim());
    let line = line.trim();
    let clipped: String = line.chars().take(60).collect();
    if clipped.chars().count() < line.chars().count() || rest.lines().nth(1).is_some() {
        format!("{}…", clipped.trim_end())
    } else {
        clipped
    }
}

/// A shell command's steps: the commands in it joined by `&&`, `||`, `;`, or a new line, where they aren't quoted.
fn steps(command: &str) -> Vec<String> {
    let mut steps = vec![String::new()];
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        let joined = match (quote, c) {
            (Some(q), _) if c == q => {
                quote = None;
                false
            }
            (Some(_), _) => false,
            (None, '\'' | '"') => {
                quote = Some(c);
                false
            }
            (None, ';' | '\n') => true,
            (None, '&' | '|') if chars.peek() == Some(&c) => {
                chars.next();
                true
            }
            _ => false,
        };
        match joined {
            true => steps.push(String::new()),
            false => steps.last_mut().unwrap().push(c),
        }
    }
    steps.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Is this word a shell variable being set, like `NODE_ENV=production`?
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// A short description of a tool call, for the "working" line, e.g. "Edited main.rs" or "Ran npm test".
fn activity(tool: &str, detail: &str) -> String {
    let file = Path::new(detail).file_name().map_or(detail.to_string(), |n| n.to_string_lossy().into_owned());
    match tool {
        "Bash" => format!("Ran {}", short_command(detail)),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => format!("Edited {file}"),
        "Read" => format!("Read {file}"),
        "Grep" | "Glob" => "Searched the code".into(),
        "WebFetch" | "WebSearch" => "Looked something up".into(),
        "Task" | "Agent" => "Ran a subagent".into(),
        "" => "Working".into(),
        other => format!("Used {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, extra: Value) -> Value {
        let mut e = json!({ "session_id": "s1", "hook_event_name": name, "cwd": "/Users/me/Code/site" });
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        e
    }

    fn status(sessions: &Sessions) -> Status {
        sessions.sessions["s1"].status
    }

    #[test]
    fn follows_a_session_through_a_turn() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("SessionStart", json!({})));
        assert!(status(&sessions) == Status::Idle);

        sessions.apply(&event("UserPromptSubmit", json!({})));
        assert!(status(&sessions) == Status::Working);

        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "detail": "npm test" })));
        assert!(status(&sessions) == Status::Permission);
        assert_eq!(sessions.waiting(), (1, 0));

        sessions.apply(&event("PostToolUse", json!({ "tool_name": "Bash", "detail": "npm test" })));
        assert!(status(&sessions) == Status::Working);
        assert!(sessions.sessions["s1"].request.is_none());

        sessions.apply(&event("Stop", json!({ "last_assistant_message": "All done." })));
        assert!(status(&sessions) == Status::Waiting);
        assert_eq!(sessions.waiting(), (0, 1));

        sessions.apply(&event("SessionEnd", json!({})));
        assert!(sessions.sessions.is_empty());
    }

    #[test]
    fn a_denied_request_clears_when_the_turn_ends() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "detail": "rm -rf build" })));
        sessions.apply(&event("Stop", json!({ "last_assistant_message": "Okay, I won't." })));
        assert!(status(&sessions) == Status::Waiting);
        assert!(sessions.sessions["s1"].request.is_none());
    }

    #[test]
    fn a_session_at_work_is_in_the_list_until_its_message_goes_in() {
        let mut sessions = Sessions::default();
        let now = Local::now().timestamp_millis();
        let list = |sessions: &Sessions| sessions.pending(chrono::Duration::minutes(2), &HashMap::new());
        sessions.apply(&event("UserPromptSubmit", json!({ "at": now - 60_000 })));
        sessions.apply(&event("PostToolUse", json!({ "at": now, "tool_name": "Bash", "detail": "npm test" })));
        let row = &list(&sessions)[0];
        assert_eq!(row["state"], "working");
        assert_eq!(row["what"], "Ran npm test");
        assert_eq!(row["since"], now - 60_000);

        // Handed the user's message at the end of the turn, it isn't done yet
        let change = sessions.apply(&event("Stop", json!({ "at": now, "interjected": true })));
        assert!(change.is_none());
        assert!(status(&sessions) == Status::Working);
        assert_eq!(list(&sessions)[0]["what"], "Reading your message");
    }

    #[test]
    fn a_message_from_the_list_with_the_users_next_one_still_ends_the_wait() {
        let mut sessions = Sessions::default();
        let request = json!({ "tool_name": "Bash", "detail": "ls", "request_id": "p1", "hold": 120 });
        sessions.apply(&event("PermissionRequest", request));
        assert_eq!(sessions.held.len(), 1);
        sessions.apply(&event("UserPromptSubmit", json!({ "interjected": true })));
        assert!(sessions.held.is_empty());
    }

    #[test]
    fn a_finished_turn_marked_seen_leaves_the_list_until_the_next_one() {
        let mut sessions = Sessions::default();
        let list = |sessions: &Sessions| sessions.pending(chrono::Duration::minutes(2), &HashMap::new()).len();
        sessions.apply(&event("Stop", json!({ "last_assistant_message": "Done." })));
        assert_eq!((list(&sessions), sessions.waiting()), (1, (0, 1)));
        // As the app logs it, and as it reads back after a restart
        sessions.apply(&event("HeadroomSeen", json!({})));
        assert_eq!((list(&sessions), sessions.waiting()), (0, (0, 0)));
        assert!(!sessions.mark_seen("s1"));
        sessions.apply(&event("UserPromptSubmit", json!({})));
        sessions.apply(&event("Stop", json!({ "last_assistant_message": "Done again." })));
        assert_eq!(list(&sessions), 1);
    }

    #[test]
    fn the_list_leaves_out_sessions_nobody_can_message() {
        let mut sessions = Sessions::default();
        let now = Local::now().timestamp_millis();
        let list = |sessions: &Sessions| sessions.pending(chrono::Duration::minutes(2), &HashMap::new());
        // Quiet for longer than any step takes: likely interrupted
        sessions.apply(&event("UserPromptSubmit", json!({ "at": now - 45 * 60_000 })));
        assert!(list(&sessions).is_empty());
        // Run by a script
        sessions.apply(&event("PostToolUse", json!({ "at": now, "tool_name": "Read", "unattended": true })));
        assert!(status(&sessions) == Status::Working);
        assert!(list(&sessions).is_empty());
    }

    #[test]
    fn a_reply_hold_that_takes_a_message_is_at_work_again() {
        let mut sessions = Sessions::default();
        let now = Local::now().timestamp_millis();
        sessions.apply(&event("UserPromptSubmit", json!({ "at": now - 60_000 })));
        sessions.apply(&event("Stop", json!({ "at": now - 1_000, "request_id": "r1", "reply": true, "hold": 120 })));
        assert_eq!(sessions.pending(chrono::Duration::minutes(2), &HashMap::new())[0]["replyId"], "r1");

        sessions.apply(&event("HeadroomInterjected", json!({ "at": now })));
        assert!(status(&sessions) == Status::Working);
        let row = &sessions.pending(chrono::Duration::minutes(2), &HashMap::new())[0];
        assert_eq!(row["state"], "working");
        assert!(row["replyId"].is_null());
    }

    #[test]
    fn commands_are_shortened_for_the_working_line() {
        assert_eq!(short_command("cd ~/Code/site && npm test"), "npm test");
        assert_eq!(short_command("ls"), "ls");
        // The setup it's run with
        let nvm = "source ~/.nvm/nvm.sh >/dev/null 2>&1; nvm use 20 >/dev/null; npm run build && open dist";
        assert_eq!(short_command(nvm), "npm run build");
        assert_eq!(short_command("cd /Users/me/Code/app/frontend; npx tsc --noEmit"), "npx tsc --noEmit");
        assert_eq!(short_command("R=/Users/me/Code/app; export CI=1; NODE_ENV=test npm test"), "npm test");
        // Not a ; or && inside quotes
        assert_eq!(short_command("cd x && git commit -m 'one; two && three'"), "git commit -m 'one; two && three'");
        // A program by its path, and a script written into the command
        assert_eq!(short_command("/Users/me/Code/app/node_modules/.bin/tsc -p ."), "tsc -p .");
        assert_eq!(short_command("cd x; python3 - <<'EOF'\nimport json\nEOF"), "a Python script");
        assert_eq!(short_command("python3 -c \"\nimport json\""), "a Python script");
        assert_eq!(short_command("node -e \"console.log(1)\""), "node -e \"console.log(1)\"");
        assert_eq!(short_command("npm test\nnpm run lint"), "npm test");
    }

    #[test]
    fn a_question_needs_an_answer_until_its_answered() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PreToolUse", json!({ "tool_name": "AskUserQuestion", "detail": "Which color?" })));
        assert!(status(&sessions) == Status::Question);
        assert_eq!(sessions.waiting(), (1, 0));
        sessions.apply(&event("PostToolUse", json!({ "tool_name": "AskUserQuestion" })));
        assert!(status(&sessions) == Status::Working);
    }

    #[test]
    fn finishing_reports_how_long_it_ran() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("UserPromptSubmit", json!({ "at": 1_000_000 })));
        let change = sessions.apply(&event("Stop", json!({ "at": 1_090_000, "last_assistant_message": "Done." })));
        let Some(Change { kind: ChangeKind::Finished { reply, ran }, project, .. }) = change else {
            panic!("no change")
        };
        assert_eq!(project, "site");
        assert_eq!(reply.as_deref(), Some("Done."));
        assert_eq!(ran, Some(chrono::Duration::seconds(90)));
    }

    #[test]
    fn staying_in_the_same_state_isnt_news() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("UserPromptSubmit", json!({})));
        assert!(sessions.apply(&event("PostToolUse", json!({ "tool_name": "Read" }))).is_none());
    }

    #[test]
    fn answering_a_held_request_gets_it_working_again() {
        let mut sessions = Sessions::default();
        sessions.apply(&event(
            "PermissionRequest",
            json!({ "tool_name": "Bash", "request_id": "r1", "can_session": true }),
        ));
        assert_eq!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).len(), 1);
        sessions.answered("r1");
        assert!(status(&sessions) == Status::Working);
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
    }

    #[test]
    fn a_request_the_hook_gave_up_on_isnt_held() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1" })));
        sessions.apply(&event("PermissionFallback", json!({ "request_id": "r1" })));
        assert!(status(&sessions) == Status::Permission);
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
    }

    #[test]
    fn asking_twice_in_a_row_holds_the_second_request_for_its_own_time() {
        let mut sessions = Sessions::default();
        let old = Local::now().timestamp_millis() - 10 * 60_000;
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1", "at": old })));
        // The first was answered in the terminal and Claude asked again, with nothing logged in between
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r2" })));
        let asks = sessions.held(chrono::Duration::minutes(2), &HashMap::new());
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0]["id"], "r2");
    }

    #[test]
    fn an_answer_logged_by_the_app_clears_the_request_on_replay() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1" })));
        sessions.apply(&event("HeadroomAnswered", json!({ "request_id": "r1" })));
        assert!(status(&sessions) == Status::Working);
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
    }

    #[test]
    fn a_request_that_runs_out_unanswered_is_news_again() {
        let mut sessions = Sessions::default();
        let t0 = 1_790_000_000_000i64;
        let held =
            |id: &str| json!({ "tool_name": "Bash", "request_id": id, "hold": 120, "at": t0, "detail": "npm test" });
        let late = |c: Option<Change>| matches!(c.map(|c| c.kind), Some(ChangeKind::NeedsAnswer { late: true, .. }));
        sessions.apply(&event("PermissionRequest", held("r1")));
        let ran_out = json!({ "request_id": "r1", "at": t0 + 120_000 });
        assert!(late(sessions.apply(&event("PermissionFallback", ran_out))));
        // Given up early (the session was interrupted): nothing to say
        sessions.apply(&event("PermissionRequest", held("r2")));
        assert!(!late(sessions.apply(&event("PermissionFallback", json!({ "request_id": "r2", "at": t0 + 10_000 })))));
        // Handed back from the popover: it's gone from here before the hook says so
        sessions.apply(&event("PermissionRequest", held("r3")));
        sessions.handed_back("r3");
        assert!(!late(sessions.apply(&event("PermissionFallback", json!({ "request_id": "r3", "at": t0 + 120_000 })))));
    }

    #[test]
    fn a_turn_held_open_for_a_reply_waits_quietly() {
        let mut sessions = Sessions::default();
        let t0 = 1_790_000_000_000i64;
        let finished = sessions.apply(&event(
            "Stop",
            json!({ "request_id": "r1", "reply": true, "hold": 600, "at": t0, "last_assistant_message": "Done." }),
        ));
        // Still a finished turn: blue, with its notification
        assert!(matches!(finished.map(|c| c.kind), Some(ChangeKind::Finished { .. })));
        assert!(status(&sessions) == Status::Waiting);
        assert_eq!(sessions.kind_of("r1"), Some("reply"));
        // Not in the popover, or counted as needing an answer
        assert!(sessions.held(chrono::Duration::minutes(10), &HashMap::new()).is_empty());
        assert_eq!(sessions.waiting(), (0, 1));
        // Letting go of it unanswered is nothing to tell anyone about
        let let_go = sessions.apply(&event("PermissionFallback", json!({ "request_id": "r1", "at": t0 + 600_000 })));
        assert!(let_go.is_none());
    }

    #[test]
    fn a_held_question_comes_with_its_options() {
        let mut sessions = Sessions::default();
        let question = json!({ "text": "Which?", "multiple": true, "options": [{ "label": "A" }, { "label": "B" }] });
        sessions.apply(&event(
            "PreToolUse",
            json!({ "tool_name": "AskUserQuestion", "request_id": "q1", "questions": [question, question] }),
        ));
        assert!(status(&sessions) == Status::Question);
        let asks = sessions.held(chrono::Duration::minutes(2), &HashMap::new());
        assert_eq!(asks[0]["kind"], "question");
        // What the popover shows as the question
        assert_eq!(asks[0]["questions"][0]["question"], "Which?");
        assert_eq!(asks[0]["questions"][1]["multiple"], true);
        assert_eq!(asks[0]["questions"][1]["options"][1]["label"], "B");
        // One logged before several could be held
        sessions.apply(&event(
            "PreToolUse",
            json!({ "tool_name": "AskUserQuestion", "request_id": "q2", "question": question }),
        ));
        let asks = sessions.held(chrono::Duration::minutes(2), &HashMap::new());
        assert_eq!(asks[1]["questions"][0]["question"], "Which?");
    }

    #[test]
    fn a_held_request_stays_held_while_the_session_runs_other_tools() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1" })));
        sessions.apply(&event("PostToolUse", json!({ "tool_name": "Read", "detail": "a.rs" })));
        assert_eq!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).len(), 1);
        sessions.apply(&event("Stop", json!({})));
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
    }

    #[test]
    fn a_request_is_held_for_the_time_its_hook_was_given() {
        let mut sessions = Sessions::default();
        let at = Local::now().timestamp_millis() - 90_000;
        sessions.apply(&event(
            "PermissionRequest",
            json!({ "tool_name": "Bash", "request_id": "r1", "at": at, "hold": 60 }),
        ));
        assert!(sessions.held(chrono::Duration::minutes(5), &HashMap::new()).is_empty());
        let extra = HashMap::from([("r1".to_string(), 60_000)]);
        assert_eq!(sessions.held(chrono::Duration::minutes(5), &extra).len(), 1);
    }

    #[test]
    fn a_request_whose_hook_is_gone_isnt_held() {
        let mut sessions = Sessions::default();
        let gone = std::process::Command::new("true").spawn().unwrap();
        let pid = gone.id();
        let _ = { gone }.wait();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1", "pid": pid })));
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
        let me = std::process::id();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r2", "pid": me })));
        assert_eq!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).len(), 1);
    }

    #[test]
    fn a_request_answered_in_the_terminal_is_let_go() {
        let mut sessions = Sessions::default();
        let me = std::process::id();
        sessions.apply(&event(
            "PermissionRequest",
            json!({ "tool_name": "Bash", "detail": "npm test", "request_id": "r1", "pid": me }),
        ));
        sessions.apply(&event("PostToolUse", json!({ "tool_name": "Bash", "detail": "ls" })));
        assert_eq!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).len(), 1);
        sessions.apply(&event("PostToolUse", json!({ "tool_name": "Bash", "detail": "npm test" })));
        assert!(sessions.held(chrono::Duration::minutes(2), &HashMap::new()).is_empty());
        assert_eq!(sessions.take_released(), vec!["r1".to_string()]);
        assert!(sessions.take_released().is_empty());
    }

    #[test]
    fn the_end_of_a_request_doesnt_bring_back_an_ended_session() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("PermissionRequest", json!({ "tool_name": "Bash", "request_id": "r1" })));
        sessions.apply(&event("SessionEnd", json!({})));
        sessions.apply(&event("PermissionFallback", json!({ "request_id": "r1" })));
        assert!(sessions.sessions.is_empty());
    }

    #[test]
    fn a_finished_turn_follows_the_claude_app() {
        use crate::desktop::Chat;
        let mut sessions = Sessions::default();
        sessions.apply(&event("Stop", json!({})));
        assert_eq!(sessions.waiting(), (0, 1));
        // Claude asked something in plain text: yellow, like the app
        assert!(sessions.from_app("s1", Some(Chat { needs_you: true, archived: false }), false));
        assert_eq!(sessions.waiting(), (1, 0));
        // Opened in the app: no dot
        sessions.from_app("s1", Some(Chat { needs_you: true, archived: false }), true);
        assert_eq!(sessions.waiting(), (0, 0));
        // The next turn counts again
        sessions.apply(&event("UserPromptSubmit", json!({})));
        sessions.apply(&event("Stop", json!({})));
        assert_eq!(sessions.waiting(), (0, 1));
    }

    #[test]
    fn hitting_the_limit_is_its_own_status() {
        let mut sessions = Sessions::default();
        sessions.apply(&event("UserPromptSubmit", json!({})));
        sessions.apply(&event("StopFailure", json!({ "error": "rate_limit" })));
        assert!(status(&sessions) == Status::Limited);
    }
}
