//! Reading Pricerunner's answers from 2026-11-02, and the plugin played over
//! a pipe against a fake Pricerunner.

mod host;

use std::time::Duration;

use homewright_pricerunner::api::{self, LinkError};
use homewright_pricerunner::Pricerunner;
use host::FakeHost;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const IPHONE: &str = "3397102161";
const INITIAL: &str = "/dk/api/product-detail-edge-rest/public/product-detail/v0/initial/DK";
const OFFERS: &str = "/dk/api/product-detail-edge-rest/public/product-detail/v0/offers/DK";
const SUGGEST: &str = "/dk/api/instant-search-edge-rest/public/search/suggest/DK";
const OFFERS_DE: &str = "/de/api/product-detail-edge-rest/public/product-detail/v0/offers/DE";

fn fixture(name: &str) -> Value {
    let p = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn dk() -> &'static api::Site {
    api::site("dk").unwrap()
}

#[test]
fn links() {
    let l = api::parse_link("https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser").unwrap();
    assert_eq!((l.site.map(|s| s.key), l.category.as_str(), l.id.as_str()), (Some("dk"), "1", IPHONE));
    // without https, www and the name, with a query
    let l = api::parse_link("pricerunner.se/pl/1-3607137081?q=x").unwrap();
    assert_eq!((l.site.map(|s| s.key), l.id.as_str()), (Some("se"), "3607137081"));
    let l = api::parse_link("https://www.pricerunner.com/pl/1-3607137081/Mobile-Phones/Apple-iPhone-18-Pro-Max-256GB-Burgundy-Compare-Prices").unwrap();
    assert_eq!(l.site.map(|s| s.key), Some("uk"));
    // a bare id
    let l = api::parse_link(" 3397102161 ").unwrap();
    assert_eq!((l.site, l.category.as_str(), l.id.as_str()), (None, "0", IPHONE));

    assert_eq!(api::parse_link("https://www.pricerunner.dk/cl/1/Mobiltelefoner"), Err(LinkError::Bad));
    assert_eq!(api::parse_link("https://www.elgiganten.dk/product/123"), Err(LinkError::Bad));
    assert_eq!(api::parse_link("https://www.pricerunner.de/pl/1-123"), Err(LinkError::OtherSite));

    // Klarna's sites, with the country in the path
    let l = api::parse_link("https://www.klarna.com/de/shopping/pl/cl94/3216399795/Kopfhoerer/Apple-AirPods-Pro-3/").unwrap();
    assert_eq!((l.site.map(|s| s.key), l.category.as_str(), l.id.as_str()), (Some("de"), "94", "3216399795"));
    let l = api::parse_link("klarna.com/us/shopping/pl/cl94/3216399795").unwrap();
    assert_eq!(l.site.map(|s| s.key), Some("us"));
    assert_eq!(api::parse_link("https://www.klarna.com/au/shopping/pl/cl94/3216399795/"), Err(LinkError::OtherSite));
    assert_eq!(api::parse_link("https://www.klarna.com/de/"), Err(LinkError::Bad));

    assert_eq!(api::split_links("a\nb, c  d\r\n"), ["a", "b", "c", "d"]);
}

#[test]
fn product_page() {
    let p = api::parse_initial(&fixture("initial-DK-1-3397102161")).unwrap();
    assert_eq!(p.id, IPHONE);
    assert_eq!(p.name, "Apple iPhone 16, 128GB Black");
    assert_eq!(p.category, "1");
    assert_eq!(p.brand.as_deref(), Some("Apple"));
    assert_eq!(p.category_name.as_deref(), Some("Mobiltelefoner"));
    assert_eq!(dk().product_url(&p), "https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser");
    assert_eq!(api::Product::from_json(&p.to_json()).as_ref(), Some(&p));
    // the Danish path is not used on Klarna's German site
    let p = p.from(dk());
    assert_eq!(api::site("de").unwrap().product_url(&p), "https://www.klarna.com/de/shopping/pl/cl1/3397102161/");
    assert_eq!(api::site("se").unwrap().product_url(&p), "https://www.pricerunner.se/pl/1-3397102161");
    assert!(dk().product_url(&p).ends_with("-Sammenlign-Priser"));

    // an unknown id: no product, a redirect to the category
    assert_eq!(api::parse_initial(&fixture("initial-DK-missing")), None);

    let se = api::parse_initial(&fixture("initial-SE-1-3607137081")).unwrap();
    assert_eq!(se.name, "Apple iPhone 18 Pro Max 256 GB Bourgogne");
}

#[test]
fn search() {
    let found = api::parse_search(&fixture("suggest-DK-iphone-16"));
    assert_eq!(found.len(), 5);
    let f = found.iter().find(|f| f.product.id == IPHONE).unwrap();
    assert_eq!(f.product.name, "Apple iPhone 16, 128GB Black");
    assert_eq!(f.product.category, "1");
    assert_eq!(f.price, Some(6118.0));
    assert_eq!(f.currency.as_deref(), Some("DKK"));
    assert!(api::parse_search(&json!({ "products": [], "suggestions": [] })).is_empty());
}

#[test]
fn offers() {
    let p = api::parse_offers(&fixture("offers-DK-3397102161"), dk(), false);
    // ten offers and two sponsored ones
    assert_eq!(p.offers.len(), 12);
    assert_eq!(p.count, 32);
    assert_eq!(p.highest, Some(8559.0));
    assert_eq!(p.picture.as_deref(), Some("https://owp.klarna.com/product/500x500/3202898799/Apple-iPhone-16-128GB-Black.jpg"));

    let o = p.cheapest().unwrap();
    assert_eq!(o.price, 6118.0);
    assert_eq!(o.currency, "DKK");
    assert_eq!(o.shop, "NordicElectronics");
    assert_eq!(o.shop_page.as_deref(), Some("https://www.pricerunner.dk/ri/102752/NordicElectronics"));
    assert_eq!(o.shipping, Some(49.0));
    assert_eq!(o.stock, "IN_STOCK");
    assert_eq!(o.delivery_days, Some((2, 4)));
    assert!(o.url.starts_with("https://clk.klarna.com/dk/gotostore/v1/50cefcfc"));
    assert!(p.offers.windows(2).all(|w| w[0].price <= w[1].price));

    let shops = p.by_shop();
    // CS MEGASTORE has six offers at the same price
    assert_eq!(shops.len(), 8);
    assert_eq!(shops.keys().next().map(String::as_str), Some("NordicElectronics"));
    assert_eq!(shops["Bilka"], json!(7499.0));

    // with foreign shops the count and the highest price include them
    let all = api::parse_offers(&fixture("offers-DK-3397102161"), dk(), true);
    assert_eq!((all.count, all.highest), (35, Some(8559.0)));
    // Klarna has no shop pages
    let de = api::parse_offers(&fixture("offers-DK-3397102161"), api::site("de").unwrap(), false);
    assert_eq!(de.cheapest().unwrap().shop_page, None);

    let se = api::parse_offers(&fixture("offers-SE-3607137081"), api::site("se").unwrap(), false);
    assert_eq!(se.cheapest().map(|o| (o.price, o.currency.as_str())), Some((18489.0, "SEK")));

    // all sold out
    let none = api::parse_offers(&json!({ "offers": [], "staticOffers": [], "merchants": {}, "offersSummary": { "nationalOffer": null } }), dk(), false);
    assert_eq!((none.cheapest(), none.count), (None, 0));

    assert_eq!(api::price_state(6118.0), "6118");
    assert_eq!(api::price_state(6100.0), "6100");
    assert_eq!(api::price_state(7569.35), "7569.35");
    assert_eq!(api::price_state(19.9), "19.9");
}

async fn start(server: &MockServer) -> FakeHost {
    let host = FakeHost::start(Pricerunner::new(&server.uri(), Duration::from_millis(300), Duration::ZERO));
    let dir = std::env::temp_dir().join(format!("pricerunner-test-{}-{}", std::process::id(), rand_suffix()));
    host.call("initialize", json!({ "api_version": 1, "data_dir": dir })).await.unwrap();
    host
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

fn flow(data: Value) -> Value {
    json!({ "flow_id": "f1", "source": "user", "data": data })
}

async fn serve(server: &MockServer, p: &str, status: u16, body: Value) {
    Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(status).set_body_json(body)).mount(server).await;
}

/// The iPhone's page, an empty page for 1999999999 and 204 for its offers, as Pricerunner answers.
async fn pricerunner(server: &MockServer) {
    for cat in ["0", "1"] {
        serve(server, &format!("{INITIAL}/{cat}/{IPHONE}"), 200, fixture("initial-DK-1-3397102161")).await;
        serve(server, &format!("{INITIAL}/{cat}/1999999999"), 200, fixture("initial-DK-missing")).await;
    }
    Mock::given(method("GET"))
        .and(path(format!("{OFFERS}/{IPHONE}")))
        .and(query_param("sortByPreset", "PRICE"))
        .and(query_param("af_ORIGIN", "NATIONAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("offers-DK-3397102161")))
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path(format!("{OFFERS}/1999999999"))).respond_with(ResponseTemplate::new(204)).mount(server).await;
    Mock::given(method("GET"))
        .and(path(SUGGEST))
        .and(query_param("q", "iphone 16"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("suggest-DK-iphone-16")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(SUGGEST))
        .and(query_param("q", "zzqqxx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "products": [], "suggestions": [] })))
        .mount(server)
        .await;
}

fn user(input: Value) -> Value {
    json!({ "flow": flow(json!({})), "step": "user", "input": input })
}

#[tokio::test]
async fn config_flow_with_links() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;

    let r = host.call("config_flow", json!({ "flow": flow(json!({})), "step": "user" })).await.unwrap();
    assert_eq!(r["result"]["step_id"], "user");
    let sites = r["result"]["schema"]["properties"]["sites"]["items"]["oneOf"].as_array().unwrap();
    assert_eq!(sites.len(), 16);
    assert_eq!(r["result"]["schema"]["properties"]["sites"]["default"], json!(["dk"]));
    assert_eq!(r["result"]["schema"]["properties"]["foreign"]["default"], false);
    assert_eq!(r["result"]["schema"]["properties"]["links"]["format"], "textarea");

    // nothing, a shop's link, a Swedish link on the Danish site, a product that is gone
    let r = host.call("config_flow", user(json!({ "site": "dk" }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "base": "no_products" }));
    let r = host.call("config_flow", user(json!({ "sites": [], "links": IPHONE }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "sites": "no_sites" }));
    let r = host.call("config_flow", user(json!({ "site": "dk", "links": "https://www.elgiganten.dk/x" }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "links": "bad_link" }));
    // the form keeps what was typed
    assert_eq!(r["result"]["schema"]["properties"]["links"]["default"], "https://www.elgiganten.dk/x");
    let r = host.call("config_flow", user(json!({ "site": "dk", "links": "https://www.pricerunner.se/pl/1-3607137081" }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "links": "other_site" }));
    let r = host.call("config_flow", user(json!({ "site": "dk", "links": "1999999999" }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "links": "not_found" }));

    // the same product twice, as a link and as an id
    let links = format!("https://www.pricerunner.dk/pl/1-{IPHONE}/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser\n{IPHONE}");
    let r = host.call("config_flow", user(json!({ "site": "dk", "links": links }))).await.unwrap();
    // then the price alerts, which can be left empty
    assert_eq!(r["result"]["step_id"], "alerts", "{r}");
    let props = &r["result"]["schema"]["properties"];
    assert_eq!(props[format!("max_{IPHONE}")]["title"], "Apple iPhone 16, 128GB Black (DKK)");
    assert_eq!(props["notify"]["default"], "web_push");
    let r = host.call("config_flow", alerts(r["data"].clone(), json!({}))).await.unwrap();
    assert_eq!(r["result"]["type"], "create_entry", "{r}");
    assert_eq!(r["result"]["title"], "Pricerunner pricerunner.dk");
    let data = &r["result"]["data"];
    assert_eq!(data["site"], "dk");
    assert_eq!(data["sites"], json!(["dk"]));
    assert_eq!(data["foreign"], false);
    assert_eq!(data["products"].as_array().unwrap().len(), 1);
    assert_eq!(data["products"][0]["site"], "dk");
    assert_eq!(data["products"][0]["name"], "Apple iPhone 16, 128GB Black");
    assert_eq!(data["products"][0]["brand"], "Apple");
    assert_eq!(data["max_prices"], json!({}));
    assert_eq!(data["notify"], "web_push");
}

fn alerts(data: Value, input: Value) -> Value {
    json!({ "flow": flow(data), "step": "alerts", "input": input })
}

#[tokio::test]
async fn config_flow_alert_settings() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;
    let r = host.call("config_flow", user(json!({ "site": "dk", "links": IPHONE }))).await.unwrap();
    let data = r["data"].clone();
    let max = format!("max_{IPHONE}");

    // a price that is no number, and another service without a name
    let r = host.call("config_flow", alerts(data.clone(), json!({ max.clone(): "billig", "notify": "other", "notify_service": "" }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "alerts");
    assert_eq!(r["result"]["errors"], json!({ max.clone(): "bad_price", "notify_service": "bad_service" }));
    assert_eq!(r["result"]["schema"]["properties"]["notify"]["default"], "other");

    let r = host.call("config_flow", alerts(data.clone(), json!({ max.clone(): "5 999,50", "notify": "other", "notify_service": "telegram" }))).await.unwrap();
    let d = &r["result"]["data"];
    assert_eq!(d["max_prices"], json!({ IPHONE: 5999.5 }));
    assert_eq!(d["notify"], "notify.telegram");

    let input = json!({ max.clone(): 6000, "notify": "web_push", "notify_users": ["u1", " ", "u2"] });
    let r = host.call("config_flow", alerts(data.clone(), input)).await.unwrap();
    let d = &r["result"]["data"];
    assert_eq!((d["notify"].clone(), d["notify_users"].clone()), (json!("web_push"), json!(["u1", "u2"])));
    let r = host.call("config_flow", alerts(data, json!({ "notify": "all" }))).await.unwrap();
    assert_eq!(r["result"]["data"]["notify"], "notify.notify");
}

#[tokio::test]
async fn config_flow_with_search() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;

    let r = host.call("config_flow", user(json!({ "site": "dk", "search": "zzqqxx" }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "search": "no_results" }));

    let r = host.call("config_flow", user(json!({ "site": "dk", "search": "iphone 16" }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "pick");
    let options = r["result"]["schema"]["properties"]["found"]["items"]["oneOf"].as_array().unwrap().clone();
    assert_eq!(options.len(), 5);
    assert!(options.contains(&json!({ "const": IPHONE, "title": "Apple iPhone 16, 128GB Black (6118 DKK)" })));
    let data = r["data"].clone();

    let pick = |found: Value| json!({ "flow": flow(data.clone()), "step": "pick", "input": { "found": found } });
    let r = host.call("config_flow", pick(json!([]))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "found": "no_products" }));
    assert_eq!(r["result"]["schema"]["properties"]["found"]["items"]["oneOf"].as_array().unwrap().len(), 5);

    let r = host.call("config_flow", pick(json!([IPHONE, "3406824728"]))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "alerts");
    let r = host.call("config_flow", alerts(r["data"].clone(), json!({}))).await.unwrap();
    assert_eq!(r["result"]["type"], "create_entry");
    let names: Vec<&str> = r["result"]["data"]["products"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Apple iPhone 16e, 128GB Black", "Apple iPhone 16, 128GB Black"]);
    assert_eq!(r["result"]["data"]["products"][1]["path"], "/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser");
}

fn entry(products: Value) -> Value {
    json!({ "entry_id": "e1", "domain": "pricerunner", "title": "Pricerunner", "data": { "site": "dk", "products": products }, "options": {} })
}

fn iphone() -> Value {
    api::parse_initial(&fixture("initial-DK-1-3397102161")).unwrap().to_json()
}

fn gone() -> Value {
    json!({ "id": "1999999999", "category": "1", "name": "Udgået vare" })
}

#[tokio::test]
async fn options_flow_removes_and_adds() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;
    let options = |step: &str, input: Value| {
        json!({ "flow": { "flow_id": "o1", "source": "options", "data": {}, "entry": entry(json!([iphone(), gone()])) }, "step": step, "input": input })
    };

    let r = host.call("options_flow", options("init", Value::Null)).await.unwrap();
    assert_eq!(r["result"]["step_id"], "init");
    assert_eq!(r["result"]["schema"]["properties"]["keep"]["default"], json!([IPHONE, "1999999999"]));
    assert!(r["result"]["schema"]["properties"].get("site").is_none());

    let r = host.call("options_flow", options("init", json!({ "keep": [] }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "base": "no_products" }));

    // the alert step after the products, filled from the entry
    let alert = |data: Value, input: Value| {
        let mut e = entry(json!([iphone(), gone()]));
        e["options"] = json!({ "max_prices": { IPHONE: 6000.0, "1999999999": 10.0 }, "notify": "persistent_notification" });
        json!({ "flow": { "flow_id": "o1", "source": "options", "data": data, "entry": e }, "step": "alerts", "input": input })
    };
    let r = host.call("options_flow", options("init", json!({ "keep": [IPHONE] }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "alerts");
    let r = host.call("options_flow", alert(r["data"].clone(), Value::Null)).await.unwrap();
    let props = &r["result"]["schema"]["properties"];
    assert_eq!(props[format!("max_{IPHONE}")]["default"], 6000.0);
    assert!(props.get("max_1999999999").is_none());
    assert_eq!(props["notify"]["default"], "persistent_notification");
    let r = host.call("options_flow", alert(r["data"].clone(), json!({ format!("max_{IPHONE}"): 5500, "notify": "persistent_notification" }))).await.unwrap();
    assert_eq!(r["result"]["type"], "create_entry");
    assert_eq!(r["result"]["data"]["products"].as_array().unwrap().len(), 1);
    assert_eq!(r["result"]["data"]["max_prices"], json!({ IPHONE: 5500.0 }));

    // without the field nothing is removed
    let r = host.call("options_flow", options("init", json!({ "links": IPHONE }))).await.unwrap();
    let r = host.call("options_flow", alert(r["data"].clone(), json!({}))).await.unwrap();
    assert_eq!(r["result"]["data"]["products"].as_array().unwrap().len(), 2);

    // search from the options too, keeping the product that is there
    let r = host.call("options_flow", options("init", json!({ "keep": [IPHONE], "search": "iphone 16" }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "pick");
    let data = r["data"].clone();
    let r = host
        .call("options_flow", json!({ "flow": { "flow_id": "o1", "source": "options", "data": data, "entry": entry(json!([iphone()])) }, "step": "pick", "input": { "found": ["3331957792"] } }))
        .await
        .unwrap();
    let r = host.call("options_flow", alert(r["data"].clone(), json!({}))).await.unwrap();
    let ids: Vec<&str> = r["result"]["data"]["products"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [IPHONE, "3331957792"]);
}

#[tokio::test]
async fn setup_shows_the_lowest_price() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;

    host.call("setup_entry", json!({ "entry": entry(json!([iphone(), gone()])) })).await.unwrap();
    {
        let rec = host.rec.lock().unwrap();
        assert_eq!(rec.devices.len(), 2);
        let d = &rec.devices[0];
        assert_eq!(d["name"], "Apple iPhone 16, 128GB Black");
        assert_eq!(d["manufacturer"], "Apple");
        assert_eq!(d["model"], "Mobiltelefoner");
        assert_eq!(d["configuration_url"], "https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser");
        assert_eq!(rec.devices[1]["manufacturer"], "Pricerunner");
        let (spec, _) = &rec.entities["sensor.e1_3397102161_price"];
        assert_eq!(spec["device_class"], "monetary");
        assert_eq!(spec["unit_of_measurement"], "DKK");
        assert_eq!(spec["state_class"], "measurement");
    }

    let (state, a) = host.state("sensor.e1_3397102161_price").unwrap();
    assert_eq!(state, "6118");
    assert_eq!(a["shop"], "NordicElectronics");
    assert_eq!(a["shop_page"], "https://www.pricerunner.dk/ri/102752/NordicElectronics");
    assert!(a["shop_url"].as_str().unwrap().starts_with("https://clk.klarna.com/"));
    assert_eq!(a["stock"], "in_stock");
    assert_eq!(a["shipping"], 49.0);
    assert_eq!(a["price_with_shipping"], 6167.0);
    assert_eq!(a["delivery_days"], "2-4");
    assert_eq!(a["highest_price"], 8559.0);
    assert_eq!(a["offers"], 32);
    assert_eq!(a["shops"], 8);
    assert_eq!(a["shop_prices"]["Bilka"], 7499.0);
    assert_eq!(a["cheapest"].as_array().unwrap().len(), 5);
    assert_eq!(a["cheapest"][1], json!({ "shop": "INphone.dk", "price": 6169.0, "shipping": 0.0, "stock": "in_stock", "url": a["cheapest"][1]["url"] }));
    assert_eq!(a["product"], "Apple iPhone 16, 128GB Black");
    assert_eq!(a["product_url"], "https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser");
    assert_eq!(a["entity_picture"], "https://owp.klarna.com/product/500x500/3202898799/Apple-iPhone-16-128GB-Black.jpg");
    assert_eq!(a["currency"], "DKK");

    let (state, a) = host.state("sensor.e1_3397102161_shop").unwrap();
    assert_eq!(state, "NordicElectronics");
    assert_eq!(a["price"], 6118.0);
    assert!(a["entity_picture"].as_str().unwrap().starts_with("https://assets.klarnacdn.net/"));

    // the product Pricerunner no longer has: no state at first, unavailable after three failures in all
    assert!(host.state("sensor.e1_1999999999_price").is_none());
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(host.state("sensor.e1_1999999999_price").unwrap().0, "unavailable");
    assert_eq!(host.state("sensor.e1_1999999999_shop").unwrap().0, "unavailable");
    assert_eq!(host.state("sensor.e1_3397102161_price").unwrap().0, "6118");

    host.call("unload_entry", json!({ "entry_id": "e1" })).await.unwrap();
}

#[tokio::test]
async fn setup_waits_for_a_connection() {
    let server = MockServer::start().await;
    serve(&server, &format!("{OFFERS}/{IPHONE}"), 503, json!({})).await;
    let host = start(&server).await;
    let e = host.call("setup_entry", json!({ "entry": entry(json!([iphone()])) })).await.unwrap_err();
    assert_eq!(e["data"]["kind"], "not_ready");
}

#[tokio::test]
async fn unavailable_and_back() {
    let server = MockServer::start().await;
    let ok = Mock::given(method("GET"))
        .and(path(format!("{OFFERS}/{IPHONE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("offers-DK-3397102161")))
        .up_to_n_times(1)
        .mount_as_scoped(&server)
        .await;
    let host = start(&server).await;
    host.call("setup_entry", json!({ "entry": entry(json!([iphone()])) })).await.unwrap();
    assert_eq!(host.state("sensor.e1_3397102161_price").unwrap().0, "6118");
    drop(ok);

    // Pricerunner fails: twice is not enough, three times is
    let down = Mock::given(method("GET")).and(path(format!("{OFFERS}/{IPHONE}"))).respond_with(ResponseTemplate::new(500)).mount_as_scoped(&server).await;
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert_eq!(host.state("sensor.e1_3397102161_price").unwrap().0, "6118");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(host.state("sensor.e1_3397102161_price").unwrap().0, "unavailable");
    drop(down);

    // sold out everywhere
    serve(&server, &format!("{OFFERS}/{IPHONE}"), 200, json!({ "offers": [], "staticOffers": [], "merchants": {}, "images": [] })).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (state, a) = host.state("sensor.e1_3397102161_price").unwrap();
    assert_eq!(state, "unknown");
    assert_eq!(a["offers"], 0);
    assert_eq!(host.state("sensor.e1_3397102161_shop").unwrap().0, "unknown");
}

fn with_alert(max: f64, notify: Value) -> Value {
    let mut e = entry(json!([iphone()]));
    e["options"] = json!({ "max_prices": { IPHONE: max } });
    e["options"].as_object_mut().unwrap().extend(notify.as_object().unwrap().clone());
    e
}

#[tokio::test]
async fn a_price_alert_is_sent_once() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;
    let calls = || host.rec.lock().unwrap().calls.clone();

    // 6118 kr. is under 6500: one push to the user's devices, with the product page
    let push = json!({ "notify": "web_push", "notify_users": ["u1"] });
    host.call("setup_entry", json!({ "entry": with_alert(6500.0, push.clone()) })).await.unwrap();
    let c = calls();
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!((c[0]["domain"].as_str(), c[0]["service"].as_str()), (Some("notify"), Some("web_push")));
    let d = &c[0]["data"];
    assert_eq!(d["title"], "Prisalarm: Apple iPhone 16, 128GB Black");
    assert_eq!(d["message"], "Apple iPhone 16, 128GB Black koster 6118 DKK hos NordicElectronics. Din grænse er 6500 DKK.");
    assert_eq!(d["target"], json!(["u1"]));
    assert!(d["data"]["url"].as_str().unwrap().starts_with("https://www.pricerunner.dk/pl/1-3397102161"));

    // the next updates and a restart send nothing more
    tokio::time::sleep(Duration::from_millis(700)).await;
    host.call("unload_entry", json!({ "entry_id": "e1" })).await.unwrap();
    host.call("setup_entry", json!({ "entry": with_alert(6500.0, push.clone()) })).await.unwrap();
    assert_eq!(calls().len(), 1);

    // over the maximum: nothing, and the next drop under it sends again
    host.call("setup_entry", json!({ "entry": with_alert(6000.0, push.clone()) })).await.unwrap();
    assert_eq!(calls().len(), 1);
    host.call("setup_entry", json!({ "entry": with_alert(6500.0, json!({ "notify": "persistent_notification" })) })).await.unwrap();
    let c = calls();
    assert_eq!(c.len(), 2);
    assert_eq!((c[1]["domain"].as_str(), c[1]["service"].as_str()), (Some("persistent_notification"), Some("create")));
    assert_eq!(c[1]["data"]["notification_id"], "pricerunner_e1_3397102161");

    // a new maximum the price is under sends again, here to another service
    host.call("setup_entry", json!({ "entry": with_alert(6200.0, json!({ "notify": "notify.telegram" })) })).await.unwrap();
    let c = calls();
    assert_eq!(c.len(), 3);
    assert_eq!((c[2]["domain"].as_str(), c[2]["service"].as_str()), (Some("notify"), Some("telegram")));
    host.call("unload_entry", json!({ "entry_id": "e1" })).await.unwrap();
}

/// One offer from Amazon on Klarna's German site.
fn de_offers() -> Value {
    json!({
        "offers": [{ "id": "o1", "merchantId": "2", "price": { "amount": "599.00", "currency": "EUR" }, "stockStatus": "IN_STOCK", "url": "https://clk.klarna.com/de/gotostore/v1/o1" }],
        "staticOffers": [],
        "merchants": { "2": { "id": "2", "name": "Amazon", "url": "/ri/2/Amazon/" } },
        "offersSummary": { "nationalOffer": { "count": 1, "maxPrice": { "amount": "599.00", "currency": "EUR" } } },
        "images": []
    })
}

#[tokio::test]
async fn several_countries_and_foreign_shops() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    // without the filter on the country, Amazon.de is the cheapest on pricerunner.dk
    let mut dk_all = fixture("offers-DK-3397102161");
    let amazon = json!({ "id": "amz", "merchantId": "74931", "price": { "amount": "5999.00", "currency": "DKK" }, "stockStatus": "IN_STOCK", "url": "https://clk.klarna.com/dk/gotostore/v1/amz" });
    dk_all["offers"].as_array_mut().unwrap().insert(0, amazon);
    dk_all["merchants"]["74931"] = json!({ "id": "74931", "name": "Amazon", "url": "/ri/74931/Amazon" });
    Mock::given(method("GET"))
        .and(path(format!("{OFFERS}/{IPHONE}")))
        .and(query_param_is_missing("af_ORIGIN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dk_all))
        .mount(&server)
        .await;
    serve(&server, &format!("{OFFERS_DE}/{IPHONE}"), 200, de_offers()).await;
    let host = start(&server).await;

    // two countries and foreign shops, with a maximum for each country
    let r = host.call("config_flow", user(json!({ "sites": ["de", "dk"], "foreign": true, "links": IPHONE }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "alerts", "{r}");
    let props = &r["result"]["schema"]["properties"];
    assert_eq!(props[format!("max_{IPHONE}")]["title"], "Apple iPhone 16, 128GB Black, Danmark (DKK)");
    assert_eq!(props[format!("max_{IPHONE}_de")]["title"], "Apple iPhone 16, 128GB Black, Tyskland (EUR)");
    let r = host.call("config_flow", alerts(r["data"].clone(), json!({ format!("max_{IPHONE}_de"): "650" }))).await.unwrap();
    assert_eq!(r["result"]["title"], "Pricerunner pricerunner.dk, klarna.com/de");
    let d = r["result"]["data"].clone();
    assert_eq!((d["site"].clone(), d["sites"].clone(), d["foreign"].clone()), (json!("dk"), json!(["dk", "de"]), json!(true)));
    assert_eq!(d["max_prices"], json!({ format!("{IPHONE}@de"): 650.0 }));

    let e = json!({ "entry_id": "e1", "domain": "pricerunner", "title": "Pricerunner", "data": d, "options": {} });
    host.call("setup_entry", json!({ "entry": e })).await.unwrap();
    {
        let rec = host.rec.lock().unwrap();
        // one device with a price per country
        assert_eq!(rec.devices.len(), 1);
        assert_eq!(rec.entities["sensor.e1_3397102161_price"].0["name"], "Laveste pris Danmark");
        let (spec, _) = &rec.entities["sensor.e1_3397102161_de_price"];
        assert_eq!((spec["name"].as_str(), spec["unit_of_measurement"].as_str()), (Some("Laveste pris Tyskland"), Some("EUR")));
    }
    let (state, a) = host.state("sensor.e1_3397102161_price").unwrap();
    assert_eq!((state.as_str(), a["shop"].as_str(), a["offers"].as_u64()), ("5999", Some("Amazon"), Some(35)));
    let (state, a) = host.state("sensor.e1_3397102161_de_price").unwrap();
    assert_eq!((state.as_str(), a["currency"].as_str(), a["country"].as_str()), ("599", Some("EUR"), Some("de")));
    assert_eq!(a["product_url"], "https://www.klarna.com/de/shopping/pl/cl1/3397102161/");
    assert_eq!(a["shop_page"], Value::Null);
    assert_eq!(a["attribution"], "Priser fra Klarna");
    assert_eq!(host.state("sensor.e1_3397102161_de_shop").unwrap().0, "Amazon");

    // the German price is under its maximum, the Danish one has none
    let c = host.rec.lock().unwrap().calls.clone();
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0]["data"]["message"], "Apple iPhone 16, 128GB Black koster 599 EUR hos Amazon i Tyskland. Din grænse er 650 EUR.");
    assert_eq!(c[0]["data"]["data"]["tag"], format!("pricerunner-{IPHONE}_de"));
    host.call("unload_entry", json!({ "entry_id": "e1" })).await.unwrap();
}

#[tokio::test]
async fn options_flow_changes_countries() {
    let server = MockServer::start().await;
    pricerunner(&server).await;
    let host = start(&server).await;
    // an entry from before several countries: `site` and no `sites`
    let mut e = entry(json!([iphone()]));
    e["options"] = json!({ "max_prices": { IPHONE: 6000.0 } });
    let call = |step: &str, data: Value, input: Value| json!({ "flow": { "flow_id": "o1", "source": "options", "data": data, "entry": e }, "step": step, "input": input });

    let r = host.call("options_flow", call("init", json!({}), Value::Null)).await.unwrap();
    let props = &r["result"]["schema"]["properties"];
    assert_eq!((props["sites"]["default"].clone(), props["foreign"]["default"].clone()), (json!(["dk"]), json!(false)));

    let r = host.call("options_flow", call("init", json!({}), json!({ "sites": [] }))).await.unwrap();
    assert_eq!(r["result"]["errors"], json!({ "sites": "no_sites" }));

    let r = host.call("options_flow", call("init", json!({}), json!({ "sites": ["dk", "se"], "foreign": true }))).await.unwrap();
    assert_eq!(r["result"]["step_id"], "alerts", "{r}");
    let props = &r["result"]["schema"]["properties"];
    // the old maximum stays with Denmark
    assert_eq!(props[format!("max_{IPHONE}")]["default"], 6000.0);
    assert_eq!(props[format!("max_{IPHONE}_se")]["title"], "Apple iPhone 16, 128GB Black, Sverige (SEK)");
    let input = json!({ format!("max_{IPHONE}"): 6000, format!("max_{IPHONE}_se"): 7000 });
    let r = host.call("options_flow", call("alerts", r["data"].clone(), input)).await.unwrap();
    let d = &r["result"]["data"];
    assert_eq!((d["sites"].clone(), d["foreign"].clone()), (json!(["dk", "se"]), json!(true)));
    assert_eq!(d["max_prices"], json!({ IPHONE: 6000.0, format!("{IPHONE}@se"): 7000.0 }));
    assert!(d.get("site").is_none());

    // without Denmark only the Swedish field is left
    let r = host.call("options_flow", call("init", json!({}), json!({ "sites": ["se"] }))).await.unwrap();
    let props = r["result"]["schema"]["properties"].as_object().unwrap();
    assert!(props.contains_key(&format!("max_{IPHONE}_se")));
    assert!(!props.contains_key(&format!("max_{IPHONE}")));
    assert_eq!(props[&format!("max_{IPHONE}_se")]["title"], "Apple iPhone 16, 128GB Black (SEK)");
}

/// Against the real Pricerunner: `cargo test -- --ignored live --nocapture`.
#[tokio::test]
#[ignore]
async fn live() {
    let host = FakeHost::start(Pricerunner::default());
    host.call("initialize", json!({ "api_version": 1, "data_dir": std::env::temp_dir() })).await.unwrap();
    for (site, q) in [("dk", "iphone 16"), ("se", "iphone 16"), ("uk", "iphone 16"), ("de", "iphone 16"), ("us", "airpods pro")] {
        let r = host.call("config_flow", user(json!({ "site": site, "search": q }))).await.unwrap();
        let first = &r["result"]["schema"]["properties"]["found"]["items"]["oneOf"][0];
        println!("{site}: {first}");
        let data = r["data"].clone();
        let r = host.call("config_flow", json!({ "flow": flow(data), "step": "pick", "input": { "found": [first["const"]] } })).await.unwrap();
        // the alert step holds the products
        let products = r["data"]["products"].clone();
        // the link to the product, through the product page
        let link = format!("{}{}", api::site(site).unwrap().web, products[0]["path"].as_str().unwrap());
        let r = host.call("config_flow", user(json!({ "site": site, "links": link }))).await.unwrap();
        assert_eq!(r["result"]["step_id"], "alerts", "{r}");
        let entry_id = format!("live-{site}");
        let entry = json!({ "entry_id": entry_id, "data": { "site": site, "products": products } });
        host.call("setup_entry", json!({ "entry": entry })).await.unwrap();
        let id = host.entity_ending(&format!("{}_{}_price", entry_id.replace('-', "_"), products[0]["id"].as_str().unwrap()));
        let (state, a) = host.state(&id).unwrap();
        println!("{site}: {state} {}", serde_json::to_string_pretty(&a).unwrap());
        assert!(state.parse::<f64>().is_ok(), "{state}");
        host.call("unload_entry", json!({ "entry_id": entry_id })).await.unwrap();
    }
}
