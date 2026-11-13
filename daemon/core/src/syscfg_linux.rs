use anyhow::{Result, anyhow};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::time::Duration;
use zbus::Connection;
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value};

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

static SYSTEM: tokio::sync::OnceCell<Connection> = tokio::sync::OnceCell::const_new();
static SESSION: tokio::sync::OnceCell<Connection> = tokio::sync::OnceCell::const_new();

async fn system() -> Result<&'static Connection> {
    SYSTEM.get_or_try_init(|| async { Connection::system().await.map_err(|e| anyhow!("The system bus isn't available: {e}")) }).await
}

pub(crate) async fn session() -> Result<&'static Connection> {
    SESSION.get_or_try_init(|| async { Connection::session().await.map_err(|e| anyhow!("The session bus isn't available: {e}")) }).await
}

async fn call<B, R>(c: &Connection, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<R>
where
    B: serde::Serialize + DynamicType,
    R: DeserializeOwned + zbus::zvariant::Type,
{
    let m = tokio::time::timeout(Duration::from_secs(25), c.call_method(Some(dest), path, Some(iface), method, body)).await.map_err(|_| anyhow!("{dest} didn't answer."))??;
    Ok(m.body().deserialize::<R>()?)
}

async fn call0<B>(c: &Connection, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<()>
where
    B: serde::Serialize + DynamicType,
{
    tokio::time::timeout(Duration::from_secs(25), c.call_method(Some(dest), path, Some(iface), method, body)).await.map_err(|_| anyhow!("{dest} didn't answer."))??;
    Ok(())
}

async fn get(c: &Connection, dest: &str, path: &str, iface: &str, name: &str) -> Result<OwnedValue> {
    call(c, dest, path, "org.freedesktop.DBus.Properties", "Get", &(iface, name)).await
}

async fn set(c: &Connection, dest: &str, path: &str, iface: &str, name: &str, v: Value<'_>) -> Result<()> {
    call0(c, dest, path, "org.freedesktop.DBus.Properties", "Set", &(iface, name, v)).await
}

async fn all(c: &Connection, dest: &str, path: &str, iface: &str) -> Result<Props> {
    call(c, dest, path, "org.freedesktop.DBus.Properties", "GetAll", &(iface,)).await
}

async fn objects(c: &Connection, dest: &str, path: &str) -> Result<Objects> {
    call(c, dest, path, "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &()).await
}

fn b(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn s(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.downcast_ref::<String>().ok()).unwrap_or_default()
}

fn f(p: &Props, k: &str) -> f64 {
    p.get(k).and_then(|v| f64::try_from(v).ok()).unwrap_or(0.0)
}

fn u(p: &Props, k: &str) -> u32 {
    p.get(k).and_then(|v| u32::try_from(v).ok()).unwrap_or(0)
}

fn x(p: &Props, k: &str) -> i64 {
    p.get(k).and_then(|v| i64::try_from(v).ok()).unwrap_or(0)
}

fn o(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.downcast_ref::<zbus::zvariant::ObjectPath>().ok()).map(|p| p.to_string()).unwrap_or_default()
}

fn norm(t: &str) -> String {
    crate::text::normalize(t)
}

fn like(want: &str, name: &str) -> bool {
    let (w, n) = (norm(want), norm(name));
    !w.is_empty() && (n == w || n.contains(&w) || w.contains(&n) && n.len() > 2 || crate::os::score(&w, &n) >= 0.8)
}

pub enum Gv<'a> {
    Bool(bool),
    Str(&'a str),
}

fn offset_size(content: usize, count: usize) -> usize {
    [1usize, 2, 4, 8].into_iter().find(|w| content + count * w <= if *w == 8 { usize::MAX } else { (1usize << (8 * w)) - 1 }).unwrap_or(8)
}

fn put_offset(out: &mut Vec<u8>, v: usize, w: usize) {
    out.extend_from_slice(&v.to_le_bytes()[..w]);
}

pub fn dconf_blob(key: &str, value: Gv<'_>) -> Result<Vec<u8>> {
    let mut entry: Vec<u8> = key.as_bytes().to_vec();
    entry.push(0);
    let key_end = entry.len();
    while !entry.len().is_multiple_of(8) {
        entry.push(0);
    }
    match value {
        Gv::Bool(b) => entry.extend_from_slice(&[b as u8, 0, b'b']),
        Gv::Str(s) => {
            entry.extend_from_slice(s.as_bytes());
            entry.extend_from_slice(&[0, 0, b's']);
        }
    }
    entry.push(0);
    let w = offset_size(entry.len(), 1);
    put_offset(&mut entry, key_end, w);
    let mut out = entry;
    let end = out.len();
    let w = offset_size(end, 1);
    put_offset(&mut out, end, w);
    Ok(out)
}

pub async fn gsetting(key: &str, value: Gv<'_>) -> Result<()> {
    let blob = dconf_blob(key, value)?;
    let c = session().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.settings");
    let r: Result<String> = call(c, "ca.desrt.dconf", "/ca/desrt/dconf/Writer/user", "ca.desrt.dconf.Writer", "Change", &(blob,)).await;
    span.end(r.map(|_| ()))
}

pub async fn night_light(on: bool) -> Result<()> {
    gsetting("/org/gnome/settings-daemon/plugins/color/night-light-enabled", Gv::Bool(on)).await
}

pub async fn dark_mode(on: bool) -> Result<()> {
    gsetting("/org/gnome/desktop/interface/color-scheme", Gv::Str(if on { "prefer-dark" } else { "default" })).await
}

pub async fn dnd(on: bool) -> Result<()> {
    gsetting("/org/gnome/desktop/notifications/show-banners", Gv::Bool(!on)).await
}

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";

pub async fn wifi(on: bool) -> Result<()> {
    let c = system().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.network");
    span.end(set(c, NM, NM_PATH, NM, "WirelessEnabled", Value::from(on)).await.map_err(|e| anyhow!("Wi-Fi couldn't be switched: {e}")))
}

pub async fn bluetooth(on: bool) -> Result<usize> {
    let c = system().await?;
    let objs = objects(c, "org.bluez", "/").await.map_err(|_| anyhow!("This computer has no Bluetooth."))?;
    let mut n = 0;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.bluetooth");
    for (path, ifaces) in &objs {
        if ifaces.contains_key("org.bluez.Adapter1") && set(c, "org.bluez", path.as_str(), "org.bluez.Adapter1", "Powered", Value::from(on)).await.is_ok() {
            n += 1;
        }
    }
    span.end_bool(n > 0);
    if n == 0 { Err(anyhow!("Bluetooth couldn't be switched. It may be turned off with a switch or in airplane mode.")) } else { Ok(n) }
}

pub async fn airplane(on: bool) -> Result<()> {
    let c = system().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.network");
    let w = set(c, NM, NM_PATH, NM, "WirelessEnabled", Value::from(!on)).await;
    let _ = set(c, NM, NM_PATH, NM, "WwanEnabled", Value::from(!on)).await;
    let bt = bluetooth(!on).await;
    span.end_bool(w.is_ok() || bt.is_ok());
    if w.is_err() && bt.is_err() { Err(anyhow!("Airplane mode couldn't be switched.")) } else { Ok(()) }
}

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub path: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

pub async fn bt_devices() -> Result<Vec<BtDevice>> {
    let c = system().await?;
    let objs = objects(c, "org.bluez", "/").await.map_err(|_| anyhow!("This computer has no Bluetooth."))?;
    let mut v: Vec<BtDevice> = objs
        .iter()
        .filter_map(|(path, ifaces)| {
            let d = ifaces.get("org.bluez.Device1")?;
            let name = if s(d, "Alias").is_empty() { s(d, "Name") } else { s(d, "Alias") };
            Some(BtDevice { path: path.to_string(), name, paired: b(d, "Paired"), connected: b(d, "Connected") })
        })
        .collect();
    v.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.cmp(&b.name)));
    Ok(v)
}

pub async fn bt_connect(want: &str, on: bool) -> Result<String> {
    let devs = bt_devices().await?;
    let generic = matches!(norm(want).as_str(), "headphones" | "my headphones" | "earbuds" | "my earbuds" | "speaker" | "my speaker" | "headset" | "my headset" | "bluetooth" | "it");
    let pick = devs.iter().filter(|d| d.paired).find(|d| like(want, &d.name)).or_else(|| if generic { devs.iter().filter(|d| d.paired && d.connected != on).find(|_| true) } else { None }).cloned();
    let Some(d) = pick else {
        let names: Vec<String> = devs.iter().filter(|d| d.paired).map(|d| d.name.clone()).collect();
        return Err(if names.is_empty() { anyhow!("No Bluetooth devices are paired yet. Pair it once in Settings.") } else { anyhow!("I couldn't find {want}. Paired devices: {}.", names.join(", ")) });
    };
    let c = system().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.bluetooth");
    span.end(call0(c, "org.bluez", &d.path, "org.bluez.Device1", if on { "Connect" } else { "Disconnect" }, &()).await.map_err(|e| anyhow!("{} didn't {}: {e}", d.name, if on { "connect" } else { "disconnect" })))?;
    Ok(d.name)
}

const LOGIN: &str = "org.freedesktop.login1";
const LOGIN_PATH: &str = "/org/freedesktop/login1";

pub async fn power(op: &str) -> Result<()> {
    if op == "logout" {
        let c = session().await?;
        let span = crate::layers::effect(crate::layers::Layer::System, "system.power");
        return span.end(call0(c, "org.gnome.SessionManager", "/org/gnome/SessionManager", "org.gnome.SessionManager", "Logout", &(1u32,)).await.map_err(|_| anyhow!("Logging out isn't available here.")));
    }
    let (can, method) = match op {
        "suspend" => ("CanSuspend", "Suspend"),
        "hibernate" => ("CanHibernate", "Hibernate"),
        "reboot" => ("CanReboot", "Reboot"),
        "poweroff" => ("CanPowerOff", "PowerOff"),
        _ => return Err(anyhow!("I don't know that power action.")),
    };
    let c = system().await?;
    let able: String = call(c, LOGIN, LOGIN_PATH, "org.freedesktop.login1.Manager", can, &()).await.unwrap_or_else(|_| "na".into());
    if able == "no" || able == "na" {
        return Err(anyhow!("This computer can't do that right now."));
    }
    let span = crate::layers::effect(crate::layers::Layer::System, "system.power");
    span.end(call0(c, LOGIN, LOGIN_PATH, "org.freedesktop.login1.Manager", method, &(true,)).await)
}

#[derive(Debug, Clone, Default)]
pub struct Battery {
    pub present: bool,
    pub percent: f64,
    pub charging: bool,
    pub full: bool,
    pub seconds: i64,
}

pub async fn battery() -> Result<Battery> {
    let c = system().await?;
    let span = crate::layers::read(crate::layers::Layer::System, "system.battery");
    let p = span.end(all(c, "org.freedesktop.UPower", "/org/freedesktop/UPower/devices/DisplayDevice", "org.freedesktop.UPower.Device").await.map_err(|_| anyhow!("The battery can't be read here.")))?;
    let state = u(&p, "State");
    let charging = state == 1;
    Ok(Battery { present: b(&p, "IsPresent"), percent: f(&p, "Percentage"), charging, full: state == 4, seconds: if charging { x(&p, "TimeToFull") } else { x(&p, "TimeToEmpty") } })
}

pub async fn power_profile(want: Option<&str>) -> Result<(String, Vec<String>)> {
    let c = system().await?;
    for (dest, path) in [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"), ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")] {
        let Ok(p) = all(c, dest, path, dest).await else { continue };
        let names: Vec<String> = p
            .get("Profiles")
            .and_then(|v| <Vec<HashMap<String, OwnedValue>>>::try_from(v.try_clone().ok()?).ok())
            .map(|list| list.iter().map(|m| s(m, "Profile")).filter(|n| !n.is_empty()).collect())
            .unwrap_or_default();
        if let Some(w) = want {
            if !names.is_empty() && !names.iter().any(|n| n == w) {
                return Err(anyhow!("This computer has no {w} mode."));
            }
            let span = crate::layers::effect(crate::layers::Layer::System, "system.power");
            span.end(set(c, dest, path, dest, "ActiveProfile", Value::from(w)).await)?;
            return Ok((w.to_owned(), names));
        }
        return Ok((s(&p, "ActiveProfile"), names));
    }
    Err(anyhow!("Power modes aren't available on this computer."))
}

async fn nm_connections(c: &Connection) -> Result<Vec<(String, HashMap<String, Props>)>> {
    let paths: Vec<OwnedObjectPath> = call(c, NM, "/org/freedesktop/NetworkManager/Settings", "org.freedesktop.NetworkManager.Settings", "ListConnections", &()).await?;
    let mut out = Vec::new();
    for p in paths {
        if let Ok(st) = call::<_, HashMap<String, Props>>(c, NM, p.as_str(), "org.freedesktop.NetworkManager.Settings.Connection", "GetSettings", &()).await {
            out.push((p.to_string(), st));
        }
    }
    Ok(out)
}

async fn nm_active(c: &Connection) -> Result<Vec<(String, Props)>> {
    let v = get(c, NM, NM_PATH, NM, "ActiveConnections").await?;
    let paths = <Vec<OwnedObjectPath>>::try_from(v).unwrap_or_default();
    let mut out = Vec::new();
    for p in paths {
        if let Ok(pr) = all(c, NM, p.as_str(), "org.freedesktop.NetworkManager.Connection.Active").await {
            out.push((p.to_string(), pr));
        }
    }
    Ok(out)
}

fn conn_field(st: &HashMap<String, Props>, group: &str, key: &str) -> String {
    st.get(group).map(|g| s(g, key)).unwrap_or_default()
}

pub async fn vpn(on: bool, want: &str) -> Result<String> {
    let c = system().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.network");
    if !on {
        let act = nm_active(c).await?;
        let vpns: Vec<&(String, Props)> = act.iter().filter(|(_, p)| b(p, "Vpn") || s(p, "Type") == "wireguard").filter(|(_, p)| want.is_empty() || like(want, &s(p, "Id"))).collect();
        let Some((path, p)) = vpns.first() else { return span.end(Err(anyhow!("No VPN is connected."))) };
        let path = zbus::zvariant::ObjectPath::try_from(path.as_str())?;
        span.end(call0(c, NM, NM_PATH, NM, "DeactivateConnection", &(path,)).await)?;
        return Ok(s(p, "Id"));
    }
    let all_conns = nm_connections(c).await?;
    let vpns: Vec<&(String, HashMap<String, Props>)> = all_conns.iter().filter(|(_, st)| matches!(conn_field(st, "connection", "type").as_str(), "vpn" | "wireguard")).collect();
    let pick = vpns.iter().find(|(_, st)| like(want, &conn_field(st, "connection", "id"))).or_else(|| if vpns.len() == 1 || want.is_empty() { vpns.first() } else { None });
    let Some((path, st)) = pick else {
        let names: Vec<String> = vpns.iter().map(|(_, st)| conn_field(st, "connection", "id")).collect();
        return span.end(Err(if names.is_empty() { anyhow!("No VPN is set up. Add one in Settings, Network.") } else { anyhow!("Which VPN? You have {}.", names.join(", ")) }));
    };
    let conn = zbus::zvariant::ObjectPath::try_from(path.as_str())?;
    let root = zbus::zvariant::ObjectPath::try_from("/")?;
    let _: OwnedObjectPath = span.end(call(c, NM, NM_PATH, NM, "ActivateConnection", &(conn, root.clone(), root)).await)?;
    Ok(conn_field(st, "connection", "id"))
}

pub async fn hotspot(on: bool) -> Result<(String, Option<String>)> {
    let c = system().await?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.network");
    if !on {
        let conns = nm_connections(c).await?;
        let aps: Vec<String> = conns.iter().filter(|(_, st)| conn_field(st, "802-11-wireless", "mode") == "ap").map(|(_, st)| conn_field(st, "connection", "uuid")).collect();
        for (path, p) in nm_active(c).await? {
            if aps.contains(&s(&p, "Uuid")) {
                let ap = zbus::zvariant::ObjectPath::try_from(path.as_str())?;
                span.end(call0(c, NM, NM_PATH, NM, "DeactivateConnection", &(ap,)).await)?;
                return Ok((s(&p, "Id"), None));
            }
        }
        return span.end(Err(anyhow!("The hotspot isn't on.")));
    }
    let devices: Vec<OwnedObjectPath> = call(c, NM, NM_PATH, NM, "GetDevices", &()).await?;
    let mut wifi_dev = None;
    for d in devices {
        if let Ok(v) = get(c, NM, d.as_str(), "org.freedesktop.NetworkManager.Device", "DeviceType").await
            && u32::try_from(v).ok() == Some(2)
        {
            wifi_dev = Some(d);
            break;
        }
    }
    let dev = wifi_dev.ok_or_else(|| anyhow!("This computer has no Wi-Fi to share."))?;
    let root = zbus::zvariant::ObjectPath::try_from("/")?;
    for (path, st) in nm_connections(c).await? {
        if conn_field(&st, "802-11-wireless", "mode") == "ap" {
            let p = zbus::zvariant::ObjectPath::try_from(path.as_str())?;
            let _: OwnedObjectPath = span.end(call(c, NM, NM_PATH, NM, "ActivateConnection", &(p, dev.clone(), root.clone())).await)?;
            return Ok((conn_field(&st, "connection", "id"), None));
        }
    }
    let host = std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_owned()).unwrap_or_else(|_| "Setu".into());
    let ssid = format!("{host} hotspot");
    let alphabet: Vec<char> = "abcdefghjkmnpqrstuvwxyz23456789".chars().collect();
    let password: String = (0..10).map(|_| alphabet[fastrand::usize(..alphabet.len())]).collect();
    let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
    settings.insert("connection", HashMap::from([("type", Value::from("802-11-wireless")), ("id", Value::from("Hotspot")), ("autoconnect", Value::from(false))]));
    settings.insert("802-11-wireless", HashMap::from([("ssid", Value::from(ssid.as_bytes().to_vec())), ("mode", Value::from("ap")), ("band", Value::from("bg"))]));
    settings.insert("802-11-wireless-security", HashMap::from([("key-mgmt", Value::from("wpa-psk")), ("psk", Value::from(password.as_str()))]));
    settings.insert("ipv4", HashMap::from([("method", Value::from("shared"))]));
    settings.insert("ipv6", HashMap::from([("method", Value::from("ignore"))]));
    let _: (OwnedObjectPath, OwnedObjectPath) = span.end(call(c, NM, NM_PATH, NM, "AddAndActivateConnection", &(settings, dev, root)).await)?;
    Ok((ssid, Some(password)))
}

type ModeT = (String, i32, i32, f64, f64, Vec<f64>, Props);
type MonitorT = ((String, String, String, String), Vec<ModeT>, Props);
type LogicalT = (i32, i32, f64, u32, bool, Vec<(String, String, String, String)>, Props);
type StateT = (u32, Vec<MonitorT>, Vec<LogicalT>, Props);
type Planned = (i32, i32, f64, u32, bool, Vec<(String, String, Props)>);
type Layout = (i32, i32, f64, u32, bool, Vec<(String, String, HashMap<String, Value<'static>>)>);
const DC: &str = "org.gnome.Mutter.DisplayConfig";
const DC_PATH: &str = "/org/gnome/Mutter/DisplayConfig";

async fn display_state() -> Result<StateT> {
    let c = session().await?;
    call(c, DC, DC_PATH, DC, "GetCurrentState", &()).await.map_err(|_| anyhow!("The display settings can't be read here."))
}

fn current_mode(m: &MonitorT) -> Option<&ModeT> {
    m.1.iter().find(|md| b(&md.6, "is-current")).or_else(|| m.1.iter().find(|md| b(&md.6, "is-preferred")))
}

pub async fn display_info() -> Result<String> {
    let st = display_state().await?;
    let mut parts = Vec::new();
    for l in &st.2 {
        for spec in &l.5 {
            if let Some(m) = st.1.iter().find(|m| m.0 == *spec)
                && let Some(md) = current_mode(m)
            {
                let name = if s(&m.2, "display-name").is_empty() { spec.0.clone() } else { s(&m.2, "display-name") };
                parts.push(format!("{name} is at {} by {}, scale {}%", md.1, md.2, (l.2 * 100.0).round()));
            }
        }
    }
    if parts.is_empty() { Err(anyhow!("No screen was found.")) } else { Ok(parts.join("; ")) }
}

pub async fn display(res: Option<(i32, i32)>, step: i32, scale: Option<f64>) -> Result<String> {
    let st = display_state().await?;
    let primary = st.2.iter().position(|l| l.4).unwrap_or(0);
    let mut logical: Vec<Planned> = Vec::new();
    let mut said = String::new();
    for (i, l) in st.2.iter().enumerate() {
        let mut mons = Vec::new();
        let mut new_scale = l.2;
        for spec in &l.5 {
            let m = st.1.iter().find(|m| m.0 == *spec).ok_or_else(|| anyhow!("A screen went away."))?;
            let cur = current_mode(m).ok_or_else(|| anyhow!("A screen has no mode."))?;
            let mut target = cur;
            if i == primary {
                if let Some((w, h)) = res {
                    target = m.1.iter().filter(|md| md.1 == w && md.2 == h).max_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal)).ok_or_else(|| anyhow!("This screen can't do {w} by {h}."))?;
                } else if step != 0 {
                    let mut sizes: Vec<(i32, i32)> = m.1.iter().map(|md| (md.1, md.2)).collect();
                    sizes.sort_by_key(|(w, h)| w * h);
                    sizes.dedup();
                    let at = sizes.iter().position(|sz| *sz == (cur.1, cur.2)).unwrap_or(0) as i32;
                    let next = (at + step).clamp(0, sizes.len() as i32 - 1) as usize;
                    let (w, h) = sizes[next];
                    if (w, h) == (cur.1, cur.2) {
                        return Err(anyhow!("The screen is already at its {} resolution.", if step > 0 { "highest" } else { "lowest" }));
                    }
                    target = m.1.iter().filter(|md| md.1 == w && md.2 == h).max_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(cur);
                }
                if let Some(want) = scale {
                    new_scale = target.5.iter().copied().min_by(|a, b| (a - want).abs().partial_cmp(&(b - want).abs()).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(l.2);
                }
                if !target.5.is_empty() && !target.5.iter().any(|sc| (sc - new_scale).abs() < 0.001) {
                    new_scale = target.5.iter().copied().min_by(|a, b| (a - new_scale).abs().partial_cmp(&(b - new_scale).abs()).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(1.0);
                }
                said = format!("{} by {} at {}%", target.1, target.2, (new_scale * 100.0).round());
            }
            mons.push((spec.0.clone(), target.0.clone(), Props::new()));
        }
        logical.push((l.0, l.1, new_scale, l.3, l.4, mons));
    }
    let c = session().await?;
    let layout: Vec<Layout> = logical.into_iter().map(|(xx, yy, sc, t, p, mons)| (xx, yy, sc, t, p, mons.into_iter().map(|(a, b2, _)| (a, b2, HashMap::new())).collect())).collect();
    let span = crate::layers::effect(crate::layers::Layer::System, "system.display");
    span.end(call0(c, DC, DC_PATH, DC, "ApplyMonitorsConfig", &(st.0, 2u32, layout, HashMap::<String, Value>::new())).await.map_err(|e| anyhow!("The screen didn't accept that: {e}")))?;
    Ok(said)
}

pub async fn eject(want: &str) -> Result<String> {
    let c = system().await?;
    let objs = objects(c, "org.freedesktop.UDisks2", "/org/freedesktop/UDisks2").await.map_err(|_| anyhow!("Drives can't be listed here."))?;
    let mut drives: Vec<(String, String, Vec<String>, Props)> = Vec::new();
    for (path, ifaces) in &objs {
        let (Some(block), Some(fs)) = (ifaces.get("org.freedesktop.UDisks2.Block"), ifaces.get("org.freedesktop.UDisks2.Filesystem")) else { continue };
        if b(block, "HintSystem") || b(block, "HintIgnore") {
            continue;
        }
        let mounted = fs.get("MountPoints").and_then(|v| <Vec<Vec<u8>>>::try_from(v.try_clone().ok()?).ok()).map(|v| !v.is_empty()).unwrap_or(false);
        if !mounted {
            continue;
        }
        let drive = o(block, "Drive");
        let Some(dp) = objs.iter().find(|(p, _)| p.as_str() == drive).and_then(|(_, i)| i.get("org.freedesktop.UDisks2.Drive")) else { continue };
        if !b(dp, "Removable") && !b(dp, "Ejectable") && s(dp, "ConnectionBus") != "usb" {
            continue;
        }
        let label = s(block, "IdLabel");
        let name = if label.is_empty() { format!("{} {}", s(dp, "Vendor"), s(dp, "Model")).trim().to_owned() } else { label };
        match drives.iter_mut().find(|d| d.0 == drive) {
            Some(d) => d.2.push(path.to_string()),
            None => drives.push((drive.clone(), name, vec![path.to_string()], dp.clone())),
        }
    }
    let generic = matches!(norm(want).as_str(), "" | "usb" | "the usb" | "usb drive" | "the usb drive" | "drive" | "the drive" | "usb stick" | "pen drive" | "disk" | "it");
    let pick = drives.iter().find(|d| like(want, &d.1)).or_else(|| if generic && drives.len() == 1 { drives.first() } else { None });
    let Some((drive, name, fss, dp)) = pick else {
        return Err(if drives.is_empty() { anyhow!("No removable drive is plugged in.") } else { anyhow!("Which drive? {}.", drives.iter().map(|d| d.1.clone()).collect::<Vec<_>>().join(", ")) });
    };
    let span = crate::layers::effect(crate::layers::Layer::System, "system.drives");
    for fsp in fss {
        call0(c, "org.freedesktop.UDisks2", fsp, "org.freedesktop.UDisks2.Filesystem", "Unmount", &(HashMap::<String, Value>::new(),)).await.map_err(|e| anyhow!("{name} is still in use: {e}"))?;
    }
    let none = HashMap::<String, Value>::new();
    let r = if b(dp, "Ejectable") {
        call0(c, "org.freedesktop.UDisks2", drive, "org.freedesktop.UDisks2.Drive", "Eject", &(none,)).await
    } else if b(dp, "CanPowerOff") {
        call0(c, "org.freedesktop.UDisks2", drive, "org.freedesktop.UDisks2.Drive", "PowerOff", &(none,)).await
    } else {
        Ok(())
    };
    span.end_bool(r.is_ok());
    Ok(name.clone())
}

pub async fn updates() -> Result<Vec<String>> {
    use futures_util::StreamExt;
    let c = system().await?;
    let tx: OwnedObjectPath = call(c, "org.freedesktop.PackageKit", "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit", "CreateTransaction", &()).await.map_err(|_| anyhow!("Updates can't be checked here."))?;
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender("org.freedesktop.PackageKit")?.path(tx.as_str())?.build();
    let mut stream = zbus::MessageStream::for_match_rule(rule, c, Some(256)).await?;
    let span = crate::layers::read(crate::layers::Layer::System, "system.updates");
    call0(c, "org.freedesktop.PackageKit", tx.as_str(), "org.freedesktop.PackageKit.Transaction", "GetUpdates", &(0u64,)).await?;
    let mut names = Vec::new();
    let done = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(Ok(m)) = stream.next().await {
            let member = m.header().member().map(|n| n.to_string()).unwrap_or_default();
            match member.as_str() {
                "Package" => {
                    if let Ok((_, id, _)) = m.body().deserialize::<(u32, String, String)>() {
                        names.push(id.split(';').next().unwrap_or_default().to_owned());
                    }
                }
                "ErrorCode" => {
                    let (_, details) = m.body().deserialize::<(u32, String)>().unwrap_or_default();
                    return Err(anyhow!("The update check failed: {details}"));
                }
                "Finished" => return Ok(()),
                _ => {}
            }
        }
        Err(anyhow!("The update check stopped."))
    })
    .await
    .map_err(|_| anyhow!("The update check took too long."))?;
    span.end(done)?;
    names.sort();
    names.dedup();
    Ok(names)
}

pub async fn clipboard() -> Result<String> {
    const RD: &str = "org.gnome.Mutter.RemoteDesktop";
    const RDS: &str = "org.gnome.Mutter.RemoteDesktop.Session";
    let c = session().await?;
    let span = crate::layers::read(crate::layers::Layer::System, "system.clipboard");
    let rd: OwnedObjectPath = call(c, RD, "/org/gnome/Mutter/RemoteDesktop", RD, "CreateSession", &()).await.map_err(|_| anyhow!("The clipboard can't be read on this desktop."))?;
    let r = async {
        call0(c, RD, rd.as_str(), RDS, "EnableClipboard", &(HashMap::<String, Value>::new(),)).await?;
        call0(c, RD, rd.as_str(), RDS, "Start", &()).await?;
        for mime in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"] {
            let Ok(fd) = call::<_, zbus::zvariant::OwnedFd>(c, RD, rd.as_str(), RDS, "SelectionRead", &(mime,)).await else { continue };
            let owned: std::os::fd::OwnedFd = fd.into();
            let text = tokio::task::spawn_blocking(move || {
                use std::io::Read;
                use std::os::fd::AsRawFd;
                let raw = owned.as_raw_fd();
                unsafe {
                    let flags = libc::fcntl(raw, libc::F_GETFL);
                    if flags >= 0 {
                        libc::fcntl(raw, libc::F_SETFL, flags & !libc::O_NONBLOCK);
                    }
                }
                let mut f = std::fs::File::from(owned);
                let mut buf = Vec::new();
                let _ = f.by_ref().take(64 * 1024).read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).into_owned()
            });
            if let Ok(Ok(t)) = tokio::time::timeout(Duration::from_secs(3), text).await {
                return Ok(t);
            }
        }
        Err(anyhow!("The clipboard is empty or holds no text."))
    }
    .await;
    let _ = call0(c, RD, rd.as_str(), RDS, "Stop", &()).await;
    span.end(r)
}

#[derive(Debug, Clone)]
pub struct Note {
    pub app: String,
    pub title: String,
    pub body: String,
    pub at: i64,
}

static NOTES: std::sync::Mutex<Vec<Note>> = std::sync::Mutex::new(Vec::new());

fn keep(n: Note) {
    if n.title.trim().is_empty() && n.body.trim().is_empty() {
        return;
    }
    let mut g = NOTES.lock().unwrap();
    let now = setu_common::protocol::now_ms();
    g.retain(|x| now - x.at < 6 * 3_600_000);
    g.push(n);
    let over = g.len().saturating_sub(50);
    g.drain(..over);
}

pub fn notes() -> Vec<Note> {
    let now = setu_common::protocol::now_ms();
    NOTES.lock().unwrap().iter().filter(|x| now - x.at < 6 * 3_600_000).cloned().collect()
}

pub fn clear_notes() {
    NOTES.lock().unwrap().clear();
}

pub fn watch_notifications() {
    tokio::spawn(async {
        use futures_util::StreamExt;
        let Ok(conn) = zbus::connection::Builder::session() else { return };
        let Ok(conn) = conn.build().await else { return };
        let rules = ["type='method_call',interface='org.freedesktop.Notifications',member='Notify'", "type='method_call',interface='org.gtk.Notifications',member='AddNotification'"];
        let Ok(proxy) = zbus::fdo::MonitoringProxy::new(&conn).await else { return };
        let mut parsed = Vec::new();
        for r in rules {
            if let Ok(m) = zbus::MatchRule::try_from(r) {
                parsed.push(m);
            }
        }
        if proxy.become_monitor(&parsed, 0).await.is_err() {
            return;
        }
        let mut stream = zbus::MessageStream::from(&conn);
        while let Some(Ok(m)) = stream.next().await {
            let h = m.header();
            let member = h.member().map(|n| n.to_string()).unwrap_or_default();
            let now = setu_common::protocol::now_ms();
            if member == "Notify" {
                if let Ok((app, _, _, title, body, _, _, _)) = m.body().deserialize::<(String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32)>() {
                    keep(Note { app, title, body: strip_markup(&body), at: now });
                }
            } else if member == "AddNotification"
                && let Ok((app, _, n)) = m.body().deserialize::<(String, String, HashMap<String, OwnedValue>)>()
            {
                keep(Note { app: app.rsplit('.').next().unwrap_or(&app).to_owned(), title: s(&n, "title"), body: strip_markup(&s(&n, "body")), at: now });
            }
        }
    });
}

fn strip_markup(t: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in t.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

pub fn addresses() -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut p = head;
        while !p.is_null() {
            let a = &*p;
            if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as i32 == libc::AF_INET && a.ifa_flags & libc::IFF_LOOPBACK as u32 == 0 {
                let sin = &*(a.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(a.ifa_name).to_string_lossy().into_owned();
                out.push((name, ip.to_string()));
            }
            p = a.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    out
}

pub fn pids_named(bins: &[String]) -> Vec<u32> {
    let uid = unsafe { libc::getuid() };
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        use std::os::unix::fs::MetadataExt;
        if e.metadata().map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim().to_lowercase();
        let exe = std::fs::read_link(e.path().join("exe")).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase())).unwrap_or_default();
        if bins.iter().any(|b| !b.is_empty() && (*b == comm || *b == exe)) {
            out.push(pid);
        }
    }
    out
}

pub fn kill(pids: &[u32]) -> Result<usize> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.process");
    let mut n = 0;
    for p in pids {
        if unsafe { libc::kill(*p as i32, libc::SIGTERM) } == 0 {
            n += 1;
        }
    }
    std::thread::sleep(Duration::from_millis(2500));
    for p in pids {
        if std::path::Path::new(&format!("/proc/{p}")).exists() {
            unsafe {
                libc::kill(*p as i32, libc::SIGKILL);
            }
        }
    }
    span.end_bool(n > 0);
    if n == 0 { Err(anyhow!("It couldn't be stopped.")) } else { Ok(n) }
}

pub async fn current_scale() -> Result<f64> {
    let st = display_state().await?;
    Ok(st.2.iter().find(|l| l.4).or_else(|| st.2.first()).map(|l| l.2).unwrap_or(1.0))
}

pub async fn set_default(role: &str, app: &crate::tools::apps::App) -> Result<String> {
    let mimes = crate::syscfg::ROLES.iter().find(|(r, _)| *r == role).map(|(_, m)| *m).ok_or_else(|| anyhow!("I don't know that kind of app."))?;
    let id = std::path::Path::new(&app.path).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| n.ends_with(".desktop")).ok_or_else(|| anyhow!("{} has no launcher I can use.", app.name))?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.defaults");
    span.end(crate::xdg::set_default(mimes, &id))?;
    Ok(format!("{} is now your default {}.", app.name, match role {
        "browser" => "browser",
        "email" => "email app",
        "music" => "music player",
        "video" => "video player",
        "photos" => "photo viewer",
        "pdf" => "PDF reader",
        _ => "text editor",
    }))
}

pub fn printers() -> Vec<String> {
    let out = std::process::Command::new("lpstat").arg("-e").output();
    let mut names: Vec<String> = out.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().replace('_', " ")).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

pub fn open_printer_settings() -> Result<()> {
    crate::xdg::spawn(&["gnome-control-center".into(), "printers".into()]).map(|_| ())
}

pub struct Startup {
    pub name: String,
    pub on: bool,
}

pub fn startup_list() -> Vec<Startup> {
    crate::xdg::startup_list().into_iter().map(|s| Startup { name: s.name, on: s.on }).collect()
}

pub fn set_startup(name: &str, app: Option<&crate::tools::apps::App>, on: bool) -> Result<String> {
    let want = norm(name);
    let list = crate::xdg::startup_list();
    let hit = list.iter().find(|s| norm(&s.name) == want || norm(s.id.trim_end_matches(".desktop")) == want || like(name, &s.name));
    let span = crate::layers::effect(crate::layers::Layer::System, "system.startup");
    if let Some(h) = hit {
        span.end(crate::xdg::set_startup(&h.id, None, on))?;
        return Ok(h.name.clone());
    }
    if !on {
        return span.end(Err(anyhow!("{} doesn't open at startup.", crate::text::sentence_case(name))));
    }
    let app = app.ok_or_else(|| anyhow!("I couldn't find an app called {name}."))?;
    let src = std::path::PathBuf::from(&app.path);
    let id = src.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| n.ends_with(".desktop")).ok_or_else(|| anyhow!("{} has no launcher I can use.", app.name))?;
    span.end(crate::xdg::set_startup(&id, Some(&src), true))?;
    Ok(app.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dconf_change_sets_match_glib() {
        let a = dconf_blob("/org/gnome/desktop/interface/color-scheme", Gv::Str("prefer-dark")).unwrap();
        let want_a: Vec<u8> = [&b"/org/gnome/desktop/interface/color-scheme\0"[..], &[0, 0, 0, 0, 0, 0], b"prefer-dark\0\0s\0", &[42, 64]].concat();
        assert_eq!(a, want_a);
        let b2 = dconf_blob("/org/gnome/settings-daemon/plugins/color/night-light-enabled", Gv::Bool(true)).unwrap();
        let want_b: Vec<u8> = [&b"/org/gnome/settings-daemon/plugins/color/night-light-enabled\0"[..], &[0, 0, 0, 1, 0, 98, 0, 61, 69]].concat();
        assert_eq!(b2, want_b);
    }

    #[test]
    fn markup_is_dropped_from_notifications() {
        assert_eq!(strip_markup("<b>Anoop</b> sent &amp; said hi"), "Anoop sent & said hi");
    }
}
