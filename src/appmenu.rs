//! The menus of a program, handed to the desktop's menu bar over D-Bus. The shell serves
//! `io.github.supchik22.Memrium.AppMenu`; a program describes its menus (`App::menus`), the runtime sends them with `Set`,
//! and a click comes back as the `Activated(app_id, id)` signal (`App::menu_action`). With no shell serving the name
//! (another desktop, a theme without a menu bar) nothing is sent and the program keeps its own controls.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use zbus::{blocking::Connection, message::Type, MatchRule};

pub const BUS: &str = "io.github.supchik22.Memrium.AppMenu";
pub const PATH: &str = "/io/github/supchik22/Memrium/AppMenu";
pub const IFACE: &str = "io.github.supchik22.Memrium.AppMenu";

/// One entry of a menu. `id` comes back in `App::menu_action`; a submenu's parent and a separator have none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: u32,
    pub label: String,
    pub key: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub sub: Vec<Item>,
    /// an icon for the entry: its name in the icon theme, or a PNG the program drew itself (not sent over the wire form
    /// below: only a menu the desktop reads from a program's own dbusmenu has them)
    pub icon: String,
    pub icon_png: Vec<u8>,
}

impl Item {
    pub fn new(id: u32, label: impl Into<String>) -> Item {
        Item { id, label: label.into(), enabled: true, ..Default::default() }
    }

    pub fn sep() -> Item {
        Item { separator: true, ..Default::default() }
    }

    /// A parent of a submenu.
    pub fn parent(label: impl Into<String>, sub: Vec<Item>) -> Item {
        Item { label: label.into(), enabled: true, sub, ..Default::default() }
    }

    /// The shortcut shown beside the label (text only; the program handles the key itself).
    pub fn key(mut self, key: &str) -> Item {
        self.key = key.into();
        self
    }

    pub fn enabled(mut self, on: bool) -> Item {
        self.enabled = on;
        self
    }

    pub fn checked(mut self, on: bool) -> Item {
        self.checked = on;
        self
    }

    /// The icon of the entry, by its name in the icon theme.
    pub fn icon(mut self, name: &str) -> Item {
        self.icon = name.into();
        self
    }
}

/// A menu of the bar ("View") and what is in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<Item>,
}

impl Menu {
    pub fn new(title: impl Into<String>, items: Vec<Item>) -> Menu {
        Menu { title: title.into(), items }
    }
}

/// The wire form: menus flattened to rows `(node, parent, label, key, id, flags)`. Nodes count from 1 in depth-first
/// order, parent 0 is the bar itself, so a row never has to hold its children.
pub type Row = (u32, u32, String, String, u32, u8);

pub const ENABLED: u8 = 1;
pub const CHECKED: u8 = 2;
pub const SEPARATOR: u8 = 4;

fn flags(i: &Item) -> u8 {
    (i.enabled as u8 * ENABLED) | (i.checked as u8 * CHECKED) | (i.separator as u8 * SEPARATOR)
}

pub fn flatten(menus: &[Menu]) -> Vec<Row> {
    fn walk(items: &[Item], parent: u32, rows: &mut Vec<Row>) {
        for i in items {
            let node = rows.len() as u32 + 1;
            rows.push((node, parent, i.label.clone(), i.key.clone(), i.id, flags(i)));
            walk(&i.sub, node, rows);
        }
    }
    let mut rows = Vec::new();
    for m in menus {
        let node = rows.len() as u32 + 1;
        rows.push((node, 0, m.title.clone(), String::new(), 0, ENABLED));
        walk(&m.items, node, &mut rows);
    }
    rows
}

pub fn unflatten(rows: &[Row]) -> Vec<Menu> {
    fn children(rows: &[Row], parent: u32) -> Vec<Item> {
        rows.iter()
            .filter(|r| r.1 == parent)
            .map(|r| Item { id: r.4, label: r.2.clone(), key: r.3.clone(), enabled: r.5 & ENABLED != 0, checked: r.5 & CHECKED != 0, separator: r.5 & SEPARATOR != 0, sub: children(rows, r.0), ..Default::default() })
            .collect()
    }
    rows.iter().filter(|r| r.1 == 0).map(|r| Menu { title: r.2.clone(), items: children(rows, r.0) }).collect()
}

/// A program's side of the connection.
pub struct Client {
    conn: Connection,
    app_id: String,
    sent: Vec<Row>,
    /// the shell came (back): send the menus again
    resend: Arc<AtomicBool>,
    present: Arc<AtomicBool>,
    clicks: Arc<Mutex<Vec<u32>>>,
}

impl Client {
    /// Connect to the session bus. `wake` is called (from another thread) when something arrives. `None` without a bus.
    pub fn connect(app_id: &str, wake: impl Fn() + Send + Clone + 'static) -> Option<Client> {
        let conn = Connection::session().ok()?;
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
        let present = Arc::new(AtomicBool::new(dbus.name_has_owner(BUS.try_into().ok()?).unwrap_or(false)));
        let resend = Arc::new(AtomicBool::new(false));
        let clicks = Arc::new(Mutex::new(Vec::new()));
        {
            let (conn, id, clicks, wake) = (conn.clone(), app_id.to_string(), clicks.clone(), wake.clone());
            std::thread::Builder::new()
                .name("appmenu-clicks".into())
                .spawn(move || {
                    let rule = MatchRule::builder().msg_type(Type::Signal).interface(IFACE).ok()?.member("Activated").ok()?.build();
                    for msg in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None).ok()?.flatten() {
                        if let Ok((app, item)) = msg.body().deserialize::<(String, u32)>() {
                            if app == id {
                                if let Ok(mut q) = clicks.lock() {
                                    q.push(item);
                                }
                                wake();
                            }
                        }
                    }
                    Some(())
                })
                .ok()?;
        }
        {
            let (conn, present, resend) = (conn.clone(), present.clone(), resend.clone());
            std::thread::Builder::new()
                .name("appmenu-owner".into())
                .spawn(move || {
                    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
                    for ev in dbus.receive_name_owner_changed_with_args(&[(0, BUS)]).ok()? {
                        let on = ev.args().map(|a| a.new_owner().is_some()).unwrap_or(false);
                        present.store(on, Ordering::SeqCst);
                        resend.store(on, Ordering::SeqCst);
                        wake();
                    }
                    Some(())
                })
                .ok()?;
        }
        Some(Client { conn, app_id: app_id.to_string(), sent: Vec::new(), resend, present, clicks })
    }

    /// A shell is serving the menu bar right now.
    pub fn available(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }

    /// Send the menus when they differ from the ones the shell has.
    pub fn set(&mut self, menus: &[Menu]) {
        if !self.available() {
            return;
        }
        let rows = flatten(menus);
        if rows == self.sent && !self.resend.swap(false, Ordering::SeqCst) {
            return;
        }
        self.resend.store(false, Ordering::SeqCst);
        if self.conn.call_method(Some(BUS), PATH, Some(IFACE), "Set", &(self.app_id.as_str(), &rows)).is_ok() {
            self.sent = rows;
        }
    }

    /// The ids chosen since the last call.
    pub fn take(&self) -> Vec<u32> {
        self.clicks.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip() {
        let menus = vec![
            Menu::new("View", vec![Item::new(1, "Simple").key("Ctrl+1").checked(true), Item::sep(), Item::parent("Places", vec![Item::new(20, "Auto"), Item::new(21, "2").enabled(false)])]),
            Menu::new("Edit", vec![Item::new(5, "Copy")]),
        ];
        assert_eq!(unflatten(&flatten(&menus)), menus);
    }
}
