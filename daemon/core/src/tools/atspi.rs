use crate::os::{self, Do, Node, Scope, Win};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Element {
    pub n: usize,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub label: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub web: bool,
    #[serde(default)]
    pub focused: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Screen {
    pub app: String,
    pub title: String,
    pub elements: Vec<Element>,
}

fn element(i: usize, n: Node) -> Element {
    Element { n: i + 1, x: n.x, y: n.y, w: n.w, h: n.h, label: n.name, role: n.role, value: n.value, id: n.id, editable: n.editable, web: n.web, focused: n.focused }
}

fn node(e: &Element) -> Node {
    Node {
        id: e.id.clone(),
        role: e.role.clone(),
        name: e.label.clone(),
        value: e.value.clone(),
        x: e.x,
        y: e.y,
        w: e.w,
        h: e.h,
        focused: e.focused,
        checked: None,
        editable: e.editable,
        enabled: true,
        web: e.web,
        selected: false,
    }
}

pub async fn target() -> Result<Win> {
    os::working().await
}

pub async fn snapshot(limit: usize) -> Result<Screen> {
    let win = target().await?;
    snapshot_of(&win, limit).await
}

pub async fn snapshot_of(win: &Win, limit: usize) -> Result<Screen> {
    os::wake(win).await;
    let mut nodes = os::tree(win, Scope { limit, text: false, hidden: false }).await?;
    os::reading_order(&mut nodes);
    Ok(Screen { app: win.app.clone(), title: win.title.clone(), elements: nodes.into_iter().enumerate().map(|(i, n)| element(i, n)).collect() })
}

pub async fn click(e: &Element) -> Result<()> {
    os::act(&node(e), Do::Invoke).await
}

pub async fn select(e: &Element) -> Result<()> {
    os::act(&node(e), Do::Select).await
}

pub async fn set_text(e: &Element, text: &str) -> Result<()> {
    let win = target().await?;
    os::type_into(&win, &node(e), text, true).await.map(|_| ())
}

pub async fn set_text_in(win: &Win, e: &Element, text: &str) -> Result<()> {
    os::type_into(win, &node(e), text, true).await.map(|_| ())
}

pub async fn type_text(text: &str) -> Result<()> {
    let win = target().await?;
    match os::focused(&win).await? {
        Some(n) => os::type_into(&win, &n, text, false).await.map(|_| ()),
        None => Err(anyhow!("I can't find a text field to type into. Click into the field first.")),
    }
}

pub async fn elements() -> Result<Vec<Element>> {
    let win = os::ui_targets(false).await?.into_iter().next().ok_or_else(|| anyhow!("No window is open."))?;
    let mut v = snapshot_of(&win, 99).await?.elements;
    for e in v.iter_mut() {
        if e.label.trim().is_empty() {
            e.label = "item".into();
        }
    }
    Ok(v)
}

pub async fn active_app() -> Option<(String, String)> {
    os::front().await.ok().map(|w| (w.app, w.title))
}

pub async fn enabled() -> bool {
    os::enabled().await
}

pub async fn set_enabled(on: bool) -> Result<()> {
    os::set_enabled(on).await
}

pub fn find<'a>(screen: &'a Screen, label: &str, roles: &[&str]) -> Option<&'a Element> {
    let want = crate::text::normalize(label);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(f64, &Element)> = None;
    for e in &screen.elements {
        if !roles.is_empty() && !roles.contains(&e.role.as_str()) {
            continue;
        }
        let name = crate::text::normalize(&e.label);
        if name.is_empty() {
            continue;
        }
        let score = os::score(&want, &name);
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, e));
        }
    }
    best.filter(|(s, _)| *s >= 0.75).map(|(_, e)| e)
}
