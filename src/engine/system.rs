use std::time::Duration;

use futures_util::StreamExt;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::{
    Connection,
    zvariant::{OwnedFd, OwnedObjectPath},
};

#[derive(Clone, Copy)]
pub(super) enum Service {
    Sleep,
    Lock,
    Battery,
}

pub(super) enum Update {
    Sleep(bool, Option<tokio::sync::oneshot::Sender<()>>),
    Lock(bool),
    Battery(bool),
    Available(Service, bool),
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn get_session(&self, id: &str) -> zbus::Result<OwnedObjectPath>;
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "Type")]
    fn session_type(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait Battery {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

pub(super) fn start(updates: Sender<Update>) {
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            tokio::join!(
                follow(Service::Sleep, updates.clone()),
                follow(Service::Lock, updates.clone()),
                follow(Service::Battery, updates)
            );
        });
    });
}

/// Follows one service for the daemon's life. When it isn't on the bus, it waits for the bus
/// to say the service has appeared rather than asking again and again; when it is there but
/// following it still fails, it tries again after a wait that grows to ten minutes.
async fn follow(service: Service, updates: Sender<Update>) {
    let mut wait = Duration::from_secs(5);
    loop {
        let started = std::time::Instant::now();
        let result = match service {
            Service::Sleep => sleep(&updates).await,
            Service::Lock => locks(&updates).await,
            Service::Battery => battery(&updates).await,
        };
        if result.is_err() && updates.send(Update::Available(service, false)).is_err() {
            return;
        }
        if started.elapsed() > Duration::from_secs(60) {
            wait = Duration::from_secs(5);
        }
        if !appeared(service.bus_name()).await {
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(600));
        }
    }
}

impl Service {
    fn bus_name(self) -> &'static str {
        match self {
            Service::Sleep | Service::Lock => "org.freedesktop.login1",
            Service::Battery => "org.freedesktop.UPower",
        }
    }
}

/// Waits until `name` gets an owner on the system bus, and says whether it waited: false when
/// the name already has one, or the bus can't be asked.
async fn appeared(name: &'static str) -> bool {
    let waited = async {
        let connection = Connection::system().await.ok()?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
        let mut changes = bus
            .receive_name_owner_changed_with_args(&[(0, name)])
            .await
            .ok()?;
        let owned = zbus::names::BusName::try_from(name).ok()?;
        if bus.name_has_owner(owned).await.ok()? {
            return None;
        }
        while let Some(change) = changes.next().await {
            if change.args().is_ok_and(|args| args.new_owner().is_some()) {
                return Some(());
            }
        }
        None
    };
    waited.await.is_some()
}

fn said(err: impl std::fmt::Display) -> String {
    err.to_string()
}
fn send(updates: &Sender<Update>, update: Update) -> Result<(), String> {
    updates.send(update).map_err(said)
}

async fn hold(manager: &ManagerProxy<'_>) -> Option<OwnedFd> {
    manager
        .inhibit(
            "sleep",
            "Apolline Wallpapers",
            "Pause wallpaper playback before sleep",
            "delay",
        )
        .await
        .ok()
}

async fn sleep(updates: &Sender<Update>) -> Result<(), String> {
    let connection = Connection::system().await.map_err(said)?;
    let manager = ManagerProxy::new(&connection).await.map_err(said)?;
    let mut events = manager.receive_prepare_for_sleep().await.map_err(said)?;
    let mut inhibitor = hold(&manager).await;
    send(
        updates,
        Update::Sleep(manager.preparing_for_sleep().await.map_err(said)?, None),
    )?;
    send(updates, Update::Available(Service::Sleep, true))?;
    while let Some(event) = events.next().await {
        let sleeping = event.args().map_err(said)?.start;
        if sleeping {
            let (ack, paused) = tokio::sync::oneshot::channel();
            send(updates, Update::Sleep(true, Some(ack)))?;
            let _ = tokio::time::timeout(Duration::from_secs(1), paused).await;
            inhibitor = None;
        } else {
            if inhibitor.is_none() {
                inhibitor = hold(&manager).await;
            }
            send(updates, Update::Sleep(false, None))?;
        }
    }
    Err("sleep notification stream closed".into())
}

async fn session_path(connection: &Connection) -> Result<OwnedObjectPath, String> {
    let manager = ManagerProxy::new(connection).await.map_err(said)?;
    if let Ok(id) = std::env::var("XDG_SESSION_ID")
        && let Ok(path) = manager.get_session(&id).await
    {
        return Ok(path);
    }
    if let Ok(path) = manager.get_session_by_pid(std::process::id()).await {
        return Ok(path);
    }
    let mut candidates = Vec::new();
    for (_, uid, _, seat, path) in manager.list_sessions().await.map_err(said)? {
        if uid != unsafe { libc::geteuid() } || seat.is_empty() {
            continue;
        }
        let session = SessionProxy::builder(connection)
            .path(path.clone())
            .map_err(said)?
            .build()
            .await
            .map_err(said)?;
        if session.active().await.unwrap_or(false)
            && matches!(
                session.session_type().await.ok().as_deref(),
                Some("wayland" | "x11")
            )
        {
            candidates.push(path);
        }
    }
    if candidates.len() == 1 {
        Ok(candidates.remove(0))
    } else {
        Err("cannot identify one active graphical login session".into())
    }
}

async fn locks(updates: &Sender<Update>) -> Result<(), String> {
    let connection = Connection::system().await.map_err(said)?;
    let session = SessionProxy::builder(&connection)
        .path(session_path(&connection).await?)
        .map_err(said)?
        .build()
        .await
        .map_err(said)?;
    let mut events = session.receive_locked_hint_changed().await;
    send(
        updates,
        Update::Lock(session.locked_hint().await.map_err(said)?),
    )?;
    send(updates, Update::Available(Service::Lock, true))?;
    while let Some(event) = events.next().await {
        send(updates, Update::Lock(event.get().await.map_err(said)?))?;
    }
    Err("lock notification stream closed".into())
}

async fn battery(updates: &Sender<Update>) -> Result<(), String> {
    let connection = Connection::system().await.map_err(said)?;
    let battery = BatteryProxy::new(&connection).await.map_err(said)?;
    let mut events = battery.receive_on_battery_changed().await;
    send(
        updates,
        Update::Battery(battery.on_battery().await.map_err(said)?),
    )?;
    send(updates, Update::Available(Service::Battery, true))?;
    while let Some(event) = events.next().await {
        send(updates, Update::Battery(event.get().await.map_err(said)?))?;
    }
    Err("battery notification stream closed".into())
}
