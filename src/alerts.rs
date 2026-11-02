//! Price alerts: a maximum price per product, and a message the first time
//! the lowest price is at or below it.
//!
//! The settings live in the entry next to the products:
//! - `max_prices`: product id -> maximum price
//! - `notify`: `web_push`, `persistent_notification` or a `notify.<name>`
//!   service such as `notify.notify` or `notify.telegram`
//! - `notify_people`: `person.*` entities for `web_push`; empty means everyone
//!
//! What was sent is kept in `alerts.json` in the integration's data folder,
//! so a restart does not send the same message again. A price above the
//! maximum, or a new maximum, lets the next drop send one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thehome_plugin::{json, Entry, Host, Map, Value};

use crate::api::{self, Offer, Product};

const PUSH: &str = "web_push";
const PERSISTENT: &str = "persistent_notification";
const ALL: &str = "notify.notify";
/// The choices in the form for `notify.notify` and for a service typed in
/// by hand. No dots, so they work as translation keys.
const ALL_CHOICE: &str = "all";
const OTHER: &str = "other";

/// One file for every entry; this keeps two writers apart.
static FILE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq)]
pub enum Notify {
    /// `notify.web_push` to these people, or everyone.
    Push(Vec<String>),
    /// `persistent_notification.create`.
    Persistent,
    /// `notify.<name>`.
    Service(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alerts {
    pub max: HashMap<String, f64>,
    pub notify: Notify,
}

fn people(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => vec![],
    }
}

/// A number from the form: a number, a string with a comma or a point, or
/// nothing. `Err` when it is not a price.
fn price(v: Option<&Value>) -> Result<Option<f64>, ()> {
    let n = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(n)) => n.as_f64().ok_or(())?,
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(None),
        Some(Value::String(s)) => s.trim().replace(' ', "").replace(',', ".").parse::<f64>().map_err(|_| ())?,
        _ => return Err(()),
    };
    if n.is_finite() && n >= 0.0 {
        Ok(Some(n))
    } else {
        Err(())
    }
}

/// `notify.<name>` from `notify.telegram` or `telegram`, or None.
fn service_name(s: &str) -> Option<String> {
    let name = s.trim().strip_prefix("notify.").unwrap_or(s.trim());
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    ok.then(|| format!("notify.{name}"))
}

impl Alerts {
    pub fn from_entry(entry: &Entry) -> Self {
        Self::from_map(|k| entry.get(k))
    }

    fn from_map<'a>(get: impl Fn(&str) -> Option<&'a Value>) -> Self {
        let max = get("max_prices")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect())
            .unwrap_or_default();
        let notify = match get("notify").and_then(Value::as_str).unwrap_or(PUSH) {
            PERSISTENT => Notify::Persistent,
            s => match s.strip_prefix("notify.").and_then(|_| service_name(s)) {
                Some(svc) => Notify::Service(svc),
                None => Notify::Push(people(get("notify_people"))),
            },
        };
        Self { max, notify }
    }

    pub fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        let max: Map<String, Value> = self.max.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        m.insert("max_prices".into(), Value::Object(max));
        let (notify, people) = match &self.notify {
            Notify::Push(p) => (PUSH.to_string(), p.clone()),
            Notify::Persistent => (PERSISTENT.to_string(), vec![]),
            Notify::Service(s) => (s.clone(), vec![]),
        };
        m.insert("notify".into(), json!(notify));
        m.insert("notify_people".into(), json!(people));
        m
    }

    /// Only the products that are still followed.
    pub fn keep(mut self, products: &[Product]) -> Self {
        self.max.retain(|id, _| products.iter().any(|p| &p.id == id));
        self
    }
}

impl Default for Alerts {
    fn default() -> Self {
        Self { max: HashMap::new(), notify: Notify::Push(vec![]) }
    }
}

fn max_key(id: &str) -> String {
    format!("max_{id}")
}

/// The form: a maximum per product, and where the message goes.
pub fn schema(products: &[Product], current: &Alerts, currency: &str) -> Value {
    let mut props = Map::new();
    for p in products {
        let mut f = json!({ "type": "number", "title": format!("{} ({currency})", p.name), "minimum": 0 });
        if let Some(m) = current.max.get(&p.id) {
            f["default"] = json!(m);
        }
        props.insert(max_key(&p.id), f);
    }
    let (choice, other, who) = match &current.notify {
        Notify::Push(p) => (PUSH.to_string(), String::new(), p.clone()),
        Notify::Persistent => (PERSISTENT.to_string(), String::new(), vec![]),
        Notify::Service(s) if s == ALL => (ALL_CHOICE.to_string(), String::new(), vec![]),
        Notify::Service(s) => (OTHER.to_string(), s.clone(), vec![]),
    };
    props.insert(
        "notify".into(),
        json!({
            "type": "string", "title": "Besked med", "default": choice,
            "oneOf": [
                { "const": PUSH, "title": "Push-besked" },
                { "const": PERSISTENT, "title": "Notifikation i brugerfladen" },
                { "const": ALL_CHOICE, "title": "Alle beskedtjenester (notify.notify)" },
                { "const": OTHER, "title": "En anden tjeneste" },
            ]
        }),
    );
    props.insert(
        "notify_people".into(),
        json!({
            "type": "array", "title": "Push til", "items": { "type": "string" }, "default": who,
            "description": "Tom betyder alle med push slået til.",
            "x-selector": { "entity": { "domain": ["person"], "multiple": true } }
        }),
    );
    props.insert(
        "notify_service".into(),
        json!({
            "type": "string", "title": "Anden tjeneste", "default": other,
            "description": "Fx notify.telegram. Bruges, når \"En anden tjeneste\" er valgt."
        }),
    );
    json!({ "type": "object", "properties": props })
}

/// The settings from a sent form, or the errors for its fields.
pub fn from_input(products: &[Product], input: &Value) -> Result<Alerts, Value> {
    let mut errors = Map::new();
    let mut max = HashMap::new();
    for p in products {
        match price(input.get(max_key(&p.id))) {
            Ok(Some(n)) => {
                max.insert(p.id.clone(), n);
            }
            Ok(None) => {}
            Err(()) => {
                errors.insert(max_key(&p.id), json!("bad_price"));
            }
        }
    }
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let notify = match text("notify") {
        PERSISTENT => Notify::Persistent,
        ALL_CHOICE => Notify::Service(ALL.into()),
        OTHER => match service_name(text("notify_service")) {
            Some(s) => Notify::Service(s),
            None => {
                errors.insert("notify_service".into(), json!("bad_service"));
                Notify::Push(vec![])
            }
        },
        _ => Notify::Push(people(input.get("notify_people"))),
    };
    if errors.is_empty() {
        Ok(Alerts { max, notify })
    } else {
        Err(Value::Object(errors))
    }
}

/// The form again with what was typed, after an error.
pub fn keep_input(mut schema: Value, input: &Value) -> Value {
    if let Some(props) = schema["properties"].as_object_mut() {
        for (k, f) in props.iter_mut() {
            if let Some(v) = input.get(k).filter(|v| !v.is_null()) {
                f["default"] = v.clone();
            }
        }
    }
    schema
}

/// Where the sent messages are remembered.
pub struct Sent {
    path: PathBuf,
}

impl Sent {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("alerts.json") }
    }

    fn load(&self) -> Map<String, Value> {
        std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, m: &Map<String, Value>) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.path, Value::Object(m.clone()).to_string()) {
            crate::warn!("kunne ikke gemme {}: {e}", self.path.display());
        }
    }

    /// The maximum a message was sent for.
    fn get(&self, key: &str) -> Option<f64> {
        let _g = FILE.lock().unwrap();
        self.load().get(key).and_then(Value::as_f64)
    }

    fn set(&self, key: &str, max: Option<f64>) {
        let _g = FILE.lock().unwrap();
        let mut m = self.load();
        let changed = match max {
            Some(v) => m.insert(key.into(), json!(v)) != Some(json!(v)),
            None => m.remove(key).is_some(),
        };
        if changed {
            self.save(&m);
        }
    }

    /// Forget the products of an entry that are no longer followed or have
    /// no maximum.
    pub fn retain(&self, entry_id: &str, alerts: &Alerts) {
        let _g = FILE.lock().unwrap();
        let mut m = self.load();
        let before = m.len();
        let prefix = format!("{entry_id}/");
        m.retain(|k, _| k.strip_prefix(&prefix).is_none_or(|id| alerts.max.contains_key(id)));
        if m.len() != before {
            self.save(&m);
        }
    }
}

/// What a product's price means for its alert.
pub async fn check(host: &Host, sent: &Sent, alerts: &Alerts, entry_id: &str, product: &Product, url: &str, cheapest: Option<&Offer>) {
    let key = format!("{entry_id}/{}", product.id);
    let Some(&max) = alerts.max.get(&product.id) else { return };
    // sold out everywhere says nothing about the price
    let Some(o) = cheapest else { return };
    if o.price > max {
        sent.set(&key, None);
        return;
    }
    if sent.get(&key) == Some(max) {
        return;
    }
    let cur = &o.currency;
    let title = format!("Prisalarm: {}", product.name);
    let message = format!(
        "{} koster {} {cur} hos {}. Din grænse er {} {cur}.",
        product.name,
        api::price_state(o.price),
        o.shop,
        api::price_state(max)
    );
    match send(host, alerts, entry_id, product, url, &title, &message).await {
        Ok(()) => sent.set(&key, Some(max)),
        Err(e) => crate::warn!("prisalarm for {}: {e}", product.name),
    }
}

async fn send(host: &Host, alerts: &Alerts, entry_id: &str, product: &Product, url: &str, title: &str, message: &str) -> Result<(), thehome_plugin::Error> {
    let (domain, service, mut data) = match &alerts.notify {
        Notify::Push(people) => {
            let mut d = json!({ "title": title, "message": message, "data": { "url": url, "tag": format!("pricerunner-{}", product.id) } });
            if !people.is_empty() {
                d["target"] = json!(people);
            }
            ("notify", PUSH.to_string(), d)
        }
        Notify::Persistent => (
            PERSISTENT,
            "create".to_string(),
            json!({ "title": title, "message": format!("{message} [Se priserne]({url})"), "notification_id": format!("pricerunner_{entry_id}_{}", product.id) }),
        ),
        Notify::Service(s) => ("notify", s.trim_start_matches("notify.").to_string(), json!({ "title": title, "message": message })),
    };
    if let Notify::Service(_) = alerts.notify {
        data["data"] = json!({ "url": url });
    }
    host.call_service(domain, &service, data).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_and_services() {
        assert_eq!(price(Some(&json!(6000))), Ok(Some(6000.0)));
        assert_eq!(price(Some(&json!("5 999,50"))), Ok(Some(5999.5)));
        assert_eq!(price(Some(&json!(""))), Ok(None));
        assert_eq!(price(None), Ok(None));
        assert!(price(Some(&json!(-1))).is_err());
        assert!(price(Some(&json!("billig"))).is_err());
        assert_eq!(service_name("telegram").as_deref(), Some("notify.telegram"));
        assert_eq!(service_name(" notify.mobile_app_pixel ").as_deref(), Some("notify.mobile_app_pixel"));
        assert_eq!(service_name("notify."), None);
        assert_eq!(service_name("light.turn_on"), None);
    }

    #[test]
    fn settings_round_trip() {
        for notify in [Notify::Push(vec!["person.magnus".into()]), Notify::Persistent, Notify::Service("notify.telegram".into())] {
            let a = Alerts { max: HashMap::from([("1".to_string(), 6000.0)]), notify };
            let m = Value::Object(a.to_json());
            assert_eq!(Alerts::from_map(|k| m.get(k)), a);
        }
        assert_eq!(Alerts::from_map(|_| None), Alerts::default());
    }
}
