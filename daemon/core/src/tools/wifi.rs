#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use anyhow::{Result, anyhow};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Network {
    pub n: usize,
    pub ssid: String,
    pub secure: bool,
    pub saved: bool,
    pub signal: u8,
    pub enterprise: bool,
    #[serde(skip)]
    pub path: String,
}

#[derive(Debug)]
pub enum ConnectError {
    Rejected,
    Other(String),
}

#[derive(Debug)]
pub enum Event {
    Scanned { result: std::result::Result<Vec<Network>, String>, wanted: Option<String> },
    Connected { ssid: String, result: std::result::Result<(), ConnectError> },
}

#[cfg(target_os = "linux")]
mod nm {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    pub const NM: &str = "org.freedesktop.NetworkManager";

    pub async fn proxy<'a>(c: &'a zbus::Connection, path: &'a str, iface: &'a str) -> Result<zbus::Proxy<'a>> {
        Ok(zbus::Proxy::new(c, NM, path.to_owned(), iface.to_owned()).await?)
    }

    pub async fn conn() -> Result<zbus::Connection> {
        zbus::Connection::system().await.map_err(|_| anyhow!("Wi-Fi control needs NetworkManager, which isn't available."))
    }

    pub async fn wifi_device(c: &zbus::Connection) -> Result<String> {
        let nm = proxy(c, "/org/freedesktop/NetworkManager", NM).await?;
        let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).await.map_err(|_| anyhow!("Wi-Fi control needs NetworkManager, which isn't available."))?;
        for d in devices {
            let p = proxy(c, d.as_str(), "org.freedesktop.NetworkManager.Device").await?;
            let t: u32 = p.get_property("DeviceType").await.unwrap_or(0);
            if t == 2 {
                return Ok(d.as_str().to_owned());
            }
        }
        Err(anyhow!("This computer has no Wi-Fi adapter."))
    }

    pub async fn saved(c: &zbus::Connection) -> Result<HashMap<String, String>> {
        let s = proxy(c, "/org/freedesktop/NetworkManager/Settings", "org.freedesktop.NetworkManager.Settings").await?;
        let list: Vec<OwnedObjectPath> = s.call("ListConnections", &()).await?;
        let mut out = HashMap::new();
        for p in list {
            let cp = proxy(c, p.as_str(), "org.freedesktop.NetworkManager.Settings.Connection").await?;
            let settings: HashMap<String, HashMap<String, OwnedValue>> = match cp.call("GetSettings", &()).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let is_wifi = settings.get("connection").and_then(|c| c.get("type")).and_then(|v| <&str>::try_from(v).ok()).is_some_and(|t| t == "802-11-wireless");
            if !is_wifi {
                continue;
            }
            if let Some(ssid) = settings.get("802-11-wireless").and_then(|w| w.get("ssid")).and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok()) {
                out.insert(String::from_utf8_lossy(&ssid).into_owned(), p.as_str().to_owned());
            }
        }
        Ok(out)
    }

    pub async fn scan() -> Result<Vec<Network>> {
        let span = crate::layers::read(crate::layers::Layer::System, "system.wifi.scan");
        span.end(scan_now().await)
    }

    async fn scan_now() -> Result<Vec<Network>> {
        let c = conn().await?;
        let dev = wifi_device(&c).await?;
        let w = proxy(&c, &dev, "org.freedesktop.NetworkManager.Device.Wireless").await?;
        let before: i64 = w.get_property("LastScan").await.unwrap_or(0);
        let opts: HashMap<&str, Value> = HashMap::new();
        let _: std::result::Result<(), _> = w.call("RequestScan", &(opts,)).await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let now: i64 = w.get_property("LastScan").await.unwrap_or(0);
            if now != before {
                break;
            }
        }
        let aps: Vec<OwnedObjectPath> = w.call("GetAllAccessPoints", &()).await?;
        let saved = saved(&c).await.unwrap_or_default();
        let mut best: HashMap<String, Network> = HashMap::new();
        for ap in aps {
            let p = proxy(&c, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint").await?;
            let ssid: Vec<u8> = p.get_property("Ssid").await.unwrap_or_default();
            if ssid.is_empty() {
                continue;
            }
            let ssid = String::from_utf8_lossy(&ssid).chars().filter(|c| !c.is_control()).collect::<String>();
            let strength: u8 = p.get_property("Strength").await.unwrap_or(0);
            let flags: u32 = p.get_property("Flags").await.unwrap_or(0);
            let wpa: u32 = p.get_property("WpaFlags").await.unwrap_or(0);
            let rsn: u32 = p.get_property("RsnFlags").await.unwrap_or(0);
            let enterprise = (wpa | rsn) & 0x200 != 0;
            let n = Network { n: 0, saved: saved.contains_key(&ssid), ssid: ssid.clone(), secure: flags & 1 != 0 || wpa != 0 || rsn != 0, signal: strength, enterprise, path: ap.as_str().to_owned() };
            if best.get(&ssid).is_none_or(|b| b.signal < strength) {
                best.insert(ssid, n);
            }
        }
        let mut v: Vec<Network> = best.into_values().collect();
        v.sort_by(|a, b| b.saved.cmp(&a.saved).then(b.signal.cmp(&a.signal)));
        v.truncate(9);
        for (i, n) in v.iter_mut().enumerate() {
            n.n = i + 1;
        }
        Ok(v)
    }

    async fn wait_active(c: &zbus::Connection, active: &str) -> std::result::Result<(), ConnectError> {
        let p = proxy(c, active, "org.freedesktop.NetworkManager.Connection.Active").await.map_err(|e| ConnectError::Other(e.to_string()))?;
        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            match p.get_property::<u32>("State").await {
                Ok(2) => return Ok(()),
                Ok(4) | Err(_) => return Err(ConnectError::Rejected),
                _ => {}
            }
        }
        Err(ConnectError::Other("Connecting took too long. Try again closer to the router.".into()))
    }

    pub async fn connect(ssid: &str, net: Option<&Network>, password: Option<&str>) -> std::result::Result<(), ConnectError> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        span.end(connect_now(ssid, net, password).await)
    }

    async fn connect_now(ssid: &str, net: Option<&Network>, password: Option<&str>) -> std::result::Result<(), ConnectError> {
        let err = |e: anyhow::Error| ConnectError::Other(e.to_string());
        let c = conn().await.map_err(err)?;
        let dev = wifi_device(&c).await.map_err(err)?;
        let nm = proxy(&c, "/org/freedesktop/NetworkManager", NM).await.map_err(err)?;
        let saved = saved(&c).await.unwrap_or_default();
        if password.is_none() {
            if let Some(path) = saved.get(ssid) {
                let conn_path = OwnedObjectPath::try_from(path.as_str()).map_err(|e| ConnectError::Other(e.to_string()))?;
                let dev_path = OwnedObjectPath::try_from(dev.as_str()).map_err(|e| ConnectError::Other(e.to_string()))?;
                let root = OwnedObjectPath::try_from("/").unwrap();
                let active: OwnedObjectPath = nm.call("ActivateConnection", &(conn_path, dev_path, root)).await.map_err(|e| ConnectError::Other(e.to_string()))?;
                return wait_active(&c, active.as_str()).await;
            }
        } else if let Some(path) = saved.get(ssid)
            && let Ok(cp) = proxy(&c, path, "org.freedesktop.NetworkManager.Settings.Connection").await {
                let _: std::result::Result<(), _> = cp.call("Delete", &()).await;
            }
        let mut conn_map: HashMap<&str, Value> = HashMap::new();
        conn_map.insert("id", Value::from(ssid));
        conn_map.insert("type", Value::from("802-11-wireless"));
        let mut wifi_map: HashMap<&str, Value> = HashMap::new();
        wifi_map.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
        wifi_map.insert("mode", Value::from("infrastructure"));
        if net.is_none() {
            wifi_map.insert("hidden", Value::from(true));
        }
        let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
        settings.insert("connection", conn_map);
        settings.insert("802-11-wireless", wifi_map);
        if let Some(pw) = password {
            let mut sec: HashMap<&str, Value> = HashMap::new();
            sec.insert("key-mgmt", Value::from("wpa-psk"));
            sec.insert("psk", Value::from(pw));
            settings.insert("802-11-wireless-security", sec);
        }
        let dev_path = OwnedObjectPath::try_from(dev.as_str()).map_err(|e| ConnectError::Other(e.to_string()))?;
        let ap = net.filter(|n| !n.path.is_empty()).map(|n| n.path.clone()).unwrap_or_else(|| "/".into());
        let ap_path = OwnedObjectPath::try_from(ap.as_str()).map_err(|e| ConnectError::Other(e.to_string()))?;
        let r: std::result::Result<(OwnedObjectPath, OwnedObjectPath), _> = nm.call("AddAndActivateConnection", &(settings, dev_path, ap_path)).await;
        match r {
            Ok((_, active)) => wait_active(&c, active.as_str()).await,
            Err(e) => {
                let s = e.to_string();
                if s.contains("psk") || s.contains("802-11-wireless-security") {
                    Err(ConnectError::Rejected)
                } else {
                    Err(ConnectError::Other(format!("The network could not be joined: {s}")))
                }
            }
        }
    }

    pub async fn disconnect() -> Result<()> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        span.end(disconnect_now().await)
    }

    async fn disconnect_now() -> Result<()> {
        let c = conn().await?;
        let dev = wifi_device(&c).await?;
        let p = proxy(&c, &dev, "org.freedesktop.NetworkManager.Device").await?;
        let _: () = p.call("Disconnect", &()).await.map_err(|e| anyhow!("Wi-Fi could not be disconnected: {e}"))?;
        Ok(())
    }

    pub async fn forget(ssid: &str) -> Result<()> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        span.end(forget_now(ssid).await)
    }

    async fn forget_now(ssid: &str) -> Result<()> {
        let c = conn().await?;
        let saved = saved(&c).await?;
        let path = saved.get(ssid).ok_or_else(|| anyhow!("{ssid} is not a saved network."))?;
        let cp = proxy(&c, path, "org.freedesktop.NetworkManager.Settings.Connection").await?;
        let _: () = cp.call("Delete", &()).await?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub use nm::{connect, disconnect, forget, scan};

#[cfg(windows)]
mod wlan {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::*;
    use windows::core::{GUID, HSTRING, PCWSTR};

    struct Client(HANDLE);

    impl Drop for Client {
        fn drop(&mut self) {
            unsafe {
                WlanCloseHandle(self.0, None);
            }
        }
    }

    static ONE: Mutex<()> = Mutex::new(());
    static SEEN: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
    static READY: Condvar = Condvar::new();

    unsafe extern "system" fn heard(data: *mut L2_NOTIFICATION_DATA, _ctx: *mut core::ffi::c_void) {
        if data.is_null() {
            return;
        }
        let d = unsafe { &*data };
        if d.NotificationSource.0 & WLAN_NOTIFICATION_SOURCE_ACM.0 == 0 {
            return;
        }
        let complete = d.NotificationCode == wlan_notification_acm_connection_complete.0 as u32;
        let reason = if complete && !d.pData.is_null() && d.dwDataSize as usize >= std::mem::offset_of!(WLAN_CONNECTION_NOTIFICATION_DATA, dwFlags) {
            unsafe { (*(d.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA)).wlanReasonCode }
        } else {
            0
        };
        SEEN.lock().unwrap().push((d.NotificationCode, reason));
        READY.notify_all();
    }

    fn wide(buf: &[u16]) -> String {
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    fn open() -> Result<(Client, GUID)> {
        let (mut ver, mut h) = (0u32, HANDLE::default());
        if unsafe { WlanOpenHandle(2, None, &mut ver, &mut h) } != 0 {
            return Err(anyhow!("The Windows Wi-Fi service isn't running."));
        }
        let c = Client(h);
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if unsafe { WlanEnumInterfaces(h, None, &mut list) } != 0 || list.is_null() {
            return Err(anyhow!("This PC has no Wi-Fi adapter."));
        }
        let guid = unsafe {
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize);
            let pick = items.iter().find(|i| i.isState == wlan_interface_state_connected).or(items.first()).map(|i| i.InterfaceGuid);
            WlanFreeMemory(list as *const core::ffi::c_void);
            pick
        };
        guid.map(|g| (c, g)).ok_or_else(|| anyhow!("This PC has no Wi-Fi adapter."))
    }

    fn waiting<T>(c: &Client, start: impl FnOnce() -> Result<T>, done: impl Fn(&[(u32, u32)]) -> bool, wait: Duration) -> (Result<T>, Vec<(u32, u32)>) {
        SEEN.lock().unwrap().clear();
        let registered = unsafe { WlanRegisterNotification(c.0, WLAN_NOTIFICATION_SOURCE_ACM, true, Some(heard), None, None, None) } == 0;
        let r = start();
        if r.is_ok() && registered {
            let end = Instant::now() + wait;
            let mut g = SEEN.lock().unwrap();
            while !done(&g) {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                g = READY.wait_timeout(g, left).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
            }
        }
        if registered {
            unsafe {
                WlanRegisterNotification(c.0, WLAN_NOTIFICATION_SOURCE_NONE, true, None, None, None, None);
            }
        }
        let seen = SEEN.lock().unwrap().clone();
        (r, seen)
    }

    fn profiles(c: &Client, g: &GUID) -> Vec<String> {
        let mut list: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
        if unsafe { WlanGetProfileList(c.0, g, None, &mut list) } != 0 || list.is_null() {
            return Vec::new();
        }
        unsafe {
            let v = std::slice::from_raw_parts((*list).ProfileInfo.as_ptr(), (*list).dwNumberOfItems as usize).iter().map(|p| wide(&p.strProfileName)).collect();
            WlanFreeMemory(list as *const core::ffi::c_void);
            v
        }
    }

    struct Avail {
        net: Network,
        auth: i32,
        cipher: i32,
    }

    fn available(c: &Client, g: &GUID) -> Result<Vec<Avail>> {
        let saved = profiles(c, g);
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        if unsafe { WlanGetAvailableNetworkList(c.0, g, 0, None, &mut list) } != 0 || list.is_null() {
            return Err(anyhow!("The Wi-Fi networks couldn't be read."));
        }
        let mut best: HashMap<String, Avail> = HashMap::new();
        unsafe {
            for a in std::slice::from_raw_parts((*list).Network.as_ptr(), (*list).dwNumberOfItems as usize) {
                let n = (a.dot11Ssid.uSSIDLength as usize).min(32);
                if n == 0 {
                    continue;
                }
                let ssid: String = String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..n]).chars().filter(|c| !c.is_control()).collect();
                let profile = wide(&a.strProfileName);
                let auth = a.dot11DefaultAuthAlgorithm.0;
                let enterprise = [DOT11_AUTH_ALGO_WPA.0, DOT11_AUTH_ALGO_RSNA.0, DOT11_AUTH_ALGO_WPA3.0, DOT11_AUTH_ALGO_WPA3_ENT.0].contains(&auth);
                let signal = a.wlanSignalQuality.min(100) as u8;
                let saved_here = !profile.is_empty() || saved.contains(&ssid);
                let net = Network { n: 0, saved: saved_here, ssid: ssid.clone(), secure: a.bSecurityEnabled.as_bool(), signal, enterprise, path: if profile.is_empty() && saved_here { ssid.clone() } else { profile } };
                if best.get(&ssid).is_none_or(|b| b.net.signal < signal || (b.net.path.is_empty() && !net.path.is_empty())) {
                    best.insert(ssid, Avail { net, auth, cipher: a.dot11DefaultCipherAlgorithm.0 });
                }
            }
            WlanFreeMemory(list as *const core::ffi::c_void);
        }
        Ok(best.into_values().collect())
    }

    fn scan_now() -> Result<Vec<Network>> {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let (c, g) = open()?;
        let done = |v: &[(u32, u32)]| v.iter().any(|(code, _)| *code == wlan_notification_acm_scan_complete.0 as u32 || *code == wlan_notification_acm_scan_fail.0 as u32);
        let _ = waiting(&c, || if unsafe { WlanScan(c.0, &g, None, None, None) } == 0 { Ok(()) } else { Err(anyhow!("scan")) }, done, Duration::from_secs(4));
        let mut v: Vec<Network> = available(&c, &g)?.into_iter().map(|s| s.net).collect();
        v.sort_by(|a, b| b.saved.cmp(&a.saved).then(b.signal.cmp(&a.signal)));
        v.truncate(9);
        for (i, n) in v.iter_mut().enumerate() {
            n.n = i + 1;
        }
        Ok(v)
    }

    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    }

    pub fn profile_xml(ssid: &str, password: Option<&str>, auth: i32, cipher: i32, hidden: bool) -> String {
        let hex: String = ssid.as_bytes().iter().map(|b| format!("{b:02X}")).collect();
        let (authentication, encryption) = match (password, auth) {
            (None, _) => ("open", "none"),
            (Some(_), a) if a == DOT11_AUTH_ALGO_WPA3_SAE.0 => ("WPA3SAE", "AES"),
            (Some(_), a) if a == DOT11_AUTH_ALGO_WPA_PSK.0 => ("WPAPSK", if cipher == DOT11_CIPHER_ALGO_TKIP.0 { "TKIP" } else { "AES" }),
            (Some(_), _) => ("WPA2PSK", if cipher == DOT11_CIPHER_ALGO_TKIP.0 { "TKIP" } else { "AES" }),
        };
        let key = password.map(|p| format!("<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", xml(p))).unwrap_or_default();
        format!(
            "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID><nonBroadcast>{hidden}</nonBroadcast></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security><authEncryption><authentication>{authentication}</authentication><encryption>{encryption}</encryption><useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>",
            name = xml(ssid)
        )
    }

    fn connect_now(ssid: &str, net: Option<&Network>, password: Option<&str>) -> std::result::Result<(), ConnectError> {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let other = |e: anyhow::Error| ConnectError::Other(e.to_string());
        let (c, g) = open().map_err(other)?;
        let seen = available(&c, &g).unwrap_or_default();
        let here = seen.iter().find(|s| s.net.ssid == ssid);
        let saved = profiles(&c, &g);
        let mut profile = net.map(|n| n.path.clone()).filter(|p| !p.is_empty()).or_else(|| here.map(|s| s.net.path.clone()).filter(|p| !p.is_empty())).or_else(|| saved.iter().find(|p| *p == ssid).cloned());
        let made = password.is_some() || profile.is_none();
        if made {
            let secure = here.map(|s| s.net.secure).unwrap_or(password.is_some());
            if secure && password.is_none() {
                return Err(ConnectError::Rejected);
            }
            let body = profile_xml(ssid, password.filter(|_| secure), here.map(|s| s.auth).unwrap_or(DOT11_AUTH_ALGO_RSNA_PSK.0), here.map(|s| s.cipher).unwrap_or(DOT11_CIPHER_ALGO_CCMP.0), here.is_none());
            let mut reason = 0u32;
            let r = unsafe { WlanSetProfile(c.0, &g, 0, &HSTRING::from(body), PCWSTR::null(), true, None, &mut reason) };
            if r != 0 {
                return Err(ConnectError::Other(format!("Windows didn't accept the settings for {ssid} (code {r}, reason {reason}).")));
            }
            profile = Some(ssid.to_owned());
        }
        let name = HSTRING::from(profile.clone().unwrap_or_else(|| ssid.to_owned()));
        let params = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(name.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: if here.is_none() { WLAN_CONNECTION_HIDDEN_NETWORK } else { 0 },
        };
        let complete = wlan_notification_acm_connection_complete.0 as u32;
        let (r, got) = waiting(&c, || if unsafe { WlanConnect(c.0, &g, &params, None) } == 0 { Ok(()) } else { Err(anyhow!("connect")) }, |v| v.iter().any(|(code, _)| *code == complete), Duration::from_secs(25));
        if r.is_err() {
            return Err(ConnectError::Other(format!("Windows couldn't start joining {ssid}.")));
        }
        match got.iter().find(|(code, _)| *code == complete).map(|(_, reason)| *reason) {
            Some(WLAN_REASON_CODE_SUCCESS) => Ok(()),
            Some(reason) => {
                if made {
                    unsafe {
                        WlanDeleteProfile(c.0, &g, &name, None);
                    }
                }
                if password.is_some() { Err(ConnectError::Rejected) } else { Err(ConnectError::Other(format!("{ssid} couldn't be joined (reason {reason})."))) }
            }
            None => Err(ConnectError::Other("Joining took too long. Try again closer to the router.".into())),
        }
    }

    fn disconnect_now() -> Result<()> {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let (c, g) = open()?;
        match unsafe { WlanDisconnect(c.0, &g, None) } {
            0 => Ok(()),
            r => Err(anyhow!("Wi-Fi could not be disconnected (code {r}).")),
        }
    }

    fn forget_now(ssid: &str) -> Result<()> {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let (c, g) = open()?;
        let name = profiles(&c, &g).into_iter().find(|p| p == ssid || p.eq_ignore_ascii_case(ssid)).ok_or_else(|| anyhow!("{ssid} is not a saved network."))?;
        match unsafe { WlanDeleteProfile(c.0, &g, &HSTRING::from(name), None) } {
            0 => Ok(()),
            r => Err(anyhow!("{ssid} couldn't be forgotten (code {r}).")),
        }
    }

    pub async fn scan() -> Result<Vec<Network>> {
        let span = crate::layers::read(crate::layers::Layer::System, "system.wifi.scan");
        span.end(crate::layers::blocking(scan_now).await.map_err(|e| anyhow!(e.to_string())).and_then(|r| r))
    }

    pub async fn connect(ssid: &str, net: Option<&Network>, password: Option<&str>) -> std::result::Result<(), ConnectError> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        let (s, n, p) = (ssid.to_owned(), net.cloned(), password.map(str::to_owned));
        let r = crate::layers::blocking(move || connect_now(&s, n.as_ref(), p.as_deref())).await.unwrap_or_else(|e| Err(ConnectError::Other(e.to_string())));
        span.end(r)
    }

    pub async fn disconnect() -> Result<()> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        span.end(crate::layers::blocking(disconnect_now).await.map_err(|e| anyhow!(e.to_string())).and_then(|r| r))
    }

    pub async fn forget(ssid: &str) -> Result<()> {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.wifi");
        let s = ssid.to_owned();
        span.end(crate::layers::blocking(move || forget_now(&s)).await.map_err(|e| anyhow!(e.to_string())).and_then(|r| r))
    }
}

#[cfg(windows)]
pub use wlan::{connect, disconnect, forget, scan};

#[cfg(target_os = "macos")]
const UNSUPPORTED: &str = "Wi-Fi control by voice isn't available on this system in this version.";

#[cfg(target_os = "macos")]
pub async fn scan() -> Result<Vec<Network>> {
    Err(anyhow!(UNSUPPORTED))
}

#[cfg(target_os = "macos")]
pub async fn connect(_ssid: &str, _net: Option<&Network>, _password: Option<&str>) -> std::result::Result<(), ConnectError> {
    Err(ConnectError::Other(UNSUPPORTED.into()))
}

#[cfg(target_os = "macos")]
pub async fn disconnect() -> Result<()> {
    Err(anyhow!(UNSUPPORTED))
}

#[cfg(target_os = "macos")]
pub async fn forget(_ssid: &str) -> Result<()> {
    Err(anyhow!(UNSUPPORTED))
}
