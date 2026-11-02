//! Pricerunner's sites, its product links and the JSON its own web pages
//! fetch. None of it is a documented API, so it can change without notice.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Map, Value};

/// A Pricerunner site for one country.
#[derive(Debug, PartialEq, Eq)]
pub struct Site {
    /// Our key, stored in the entry.
    pub key: &'static str,
    pub name: &'static str,
    pub host: &'static str,
    /// The first part of the path to the site's API.
    pub prefix: &'static str,
    pub country: &'static str,
    pub language: &'static str,
    pub currency: &'static str,
}

pub const SITES: [Site; 3] = [
    Site { key: "dk", name: "Danmark (pricerunner.dk)", host: "www.pricerunner.dk", prefix: "dk", country: "DK", language: "da", currency: "DKK" },
    Site { key: "se", name: "Sverige (pricerunner.se)", host: "www.pricerunner.se", prefix: "se", country: "SE", language: "sv", currency: "SEK" },
    Site { key: "uk", name: "Storbritannien (pricerunner.com)", host: "www.pricerunner.com", prefix: "uk", country: "UK", language: "en", currency: "GBP" },
];

pub fn site(key: &str) -> Option<&'static Site> {
    SITES.iter().find(|s| s.key == key)
}

/// The site a link's host belongs to.
pub fn site_of_host(host: &str) -> Option<&'static Site> {
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    SITES.iter().find(|s| s.host.trim_start_matches("www.") == host)
}

impl Site {
    pub fn origin(&self) -> String {
        format!("https://{}", self.host)
    }

    /// The product page, with the path Pricerunner gave, or one it redirects from.
    pub fn product_url(&self, p: &Product) -> String {
        match &p.path {
            Some(path) => format!("{}{path}", self.origin()),
            None => format!("{}/pl/{}-{}", self.origin(), p.category, p.id),
        }
    }

    pub fn initial_path(&self, category: &str, id: &str) -> (String, Vec<(&'static str, String)>) {
        let path = format!("/{}/api/product-detail-edge-rest/public/product-detail/v0/initial/{}/{category}/{id}", self.prefix, self.country);
        (path, vec![("languageCode", self.language.to_string())])
    }

    /// The offers in the country, cheapest first, with the filters the product page uses.
    pub fn offers_path(&self, id: &str) -> (String, Vec<(&'static str, String)>) {
        let path = format!("/{}/api/product-detail-edge-rest/public/product-detail/v0/offers/{}/{id}", self.prefix, self.country);
        let q = vec![
            ("af_ORIGIN", "NATIONAL".to_string()),
            ("af_ITEM_CONDITION", "NEW,UNKNOWN".to_string()),
            ("af_STOCK_STATUS", "IN_STOCK,BACKORDER,PREORDER,SPECIAL_ORDER,COLLECT_IN_STORE,UNKNOWN".to_string()),
            ("sortByPreset", "PRICE".to_string()),
        ];
        (path, q)
    }

    pub fn search_path(&self, query: &str) -> (String, Vec<(&'static str, String)>) {
        let path = format!("/{}/api/instant-search-edge-rest/public/search/suggest/{}", self.prefix, self.country);
        // the site sends at most 100 characters
        (path, vec![("q", query.chars().take(100).collect())])
    }
}

/// A product to follow.
#[derive(Clone, Debug, PartialEq)]
pub struct Product {
    pub id: String,
    /// The category id from the link. `0` works too when it is not known.
    pub category: String,
    pub name: String,
    pub path: Option<String>,
    pub brand: Option<String>,
    pub category_name: Option<String>,
}

impl Product {
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "category": self.category, "name": self.name, "path": self.path, "brand": self.brand, "category_name": self.category_name })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Some(Self {
            id: s("id")?,
            category: s("category").unwrap_or_else(|| "0".into()),
            name: s("name")?,
            path: s("path"),
            brand: s("brand"),
            category_name: s("category_name"),
        })
    }
}

fn pl_path() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"/pl/(\d+)-(\d+)").unwrap())
}

/// What a user pasted: a product link, or a bare product id.
#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    /// The site of the link; `None` for a bare id.
    pub site: Option<&'static Site>,
    pub category: String,
    pub id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Not a Pricerunner product link.
    Bad,
    /// A Pricerunner site this integration does not know.
    OtherSite,
}

pub fn parse_link(s: &str) -> Result<Link, LinkError> {
    let s = s.trim();
    if !s.is_empty() && s.len() <= 12 && s.chars().all(|c| c.is_ascii_digit()) {
        return Ok(Link { site: None, category: "0".into(), id: s.to_string() });
    }
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    if !host.to_ascii_lowercase().contains("pricerunner.") {
        return Err(LinkError::Bad);
    }
    let site = site_of_host(host).ok_or(LinkError::OtherSite)?;
    let c = pl_path().captures(path).ok_or(LinkError::Bad)?;
    Ok(Link { site: Some(site), category: c[1].to_string(), id: c[2].to_string() })
}

/// The links in a text, one per line or separated by spaces or commas.
pub fn split_links(s: &str) -> Vec<&str> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect()
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
}

/// An amount such as `{"amount": "6118.00", "currency": "DKK"}`.
pub fn amount(v: &Value) -> Option<f64> {
    match v.get("amount")? {
        Value::String(s) => s.parse().ok(),
        n => n.as_f64(),
    }
}

/// The product from the product page's first answer; `None` when Pricerunner does not have it.
pub fn parse_initial(v: &Value) -> Option<Product> {
    let p = v.get("product").filter(|p| p.is_object())?;
    let path = text(p, "path");
    let from_path = path.as_deref().and_then(|p| pl_path().captures(p)).map(|c| c[1].to_string());
    let category = from_path.or_else(|| v.get("category").and_then(|c| text(c, "id"))).unwrap_or_else(|| "0".into());
    Some(Product {
        id: text(p, "id")?,
        category,
        name: text(p, "name")?,
        path,
        brand: v.get("brand").and_then(|b| text(b, "name")),
        category_name: v.get("category").and_then(|c| text(c, "name")),
    })
}

/// A product found by searching, with its lowest price.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub product: Product,
    pub price: Option<f64>,
    pub currency: Option<String>,
}

pub fn parse_search(v: &Value) -> Vec<Found> {
    let list = v.get("products").and_then(Value::as_array).into_iter().flatten();
    list.filter_map(|p| {
        let path = text(p, "url");
        let category = path.as_deref().and_then(|u| pl_path().captures(u)).map(|c| c[1].to_string()).unwrap_or_else(|| "0".into());
        let product = Product { id: text(p, "id")?, category, name: text(p, "name")?, path, brand: None, category_name: text(p, "categoryName") };
        let low = p.get("lowestPrice").filter(|l| l.is_object());
        Some(Found { product, price: low.and_then(amount), currency: low.and_then(|l| text(l, "currency")) })
    })
    .collect()
}

/// One shop's offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub id: String,
    pub shop: String,
    pub shop_id: String,
    /// The shop's page on Pricerunner.
    pub shop_page: Option<String>,
    pub shop_logo: Option<String>,
    pub price: f64,
    pub currency: String,
    pub shipping: Option<f64>,
    /// `IN_STOCK`, `BACKORDER` ...
    pub stock: String,
    /// The link to the shop, through Pricerunner.
    pub url: String,
    pub delivery_days: Option<(u64, u64)>,
}

/// What the product page shows about the prices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prices {
    /// Cheapest first.
    pub offers: Vec<Offer>,
    /// The number of offers in the country, as Pricerunner counts them.
    pub count: u64,
    pub highest: Option<f64>,
    pub picture: Option<String>,
}

/// The images come without a host and a size.
pub fn picture_url(path: &str) -> String {
    match path.strip_prefix("/product/") {
        Some(rest) => format!("https://owp.klarna.com/product/500x500/{rest}"),
        None if path.starts_with("http") => path.to_string(),
        None => format!("https://owp.klarna.com{path}"),
    }
}

pub fn parse_offers(v: &Value, site: &Site) -> Prices {
    let merchants: HashMap<&str, &Value> =
        v.get("merchants").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect()).unwrap_or_default();
    let lists = ["offers", "staticOffers"].into_iter().filter_map(|k| v.get(k).and_then(Value::as_array)).flatten();
    let mut offers: Vec<Offer> = vec![];
    for o in lists {
        let (Some(id), Some(price)) = (text(o, "id"), o.get("price").and_then(amount)) else { continue };
        if offers.iter().any(|x| x.id == id) {
            continue;
        }
        let shop_id = text(o, "merchantId").unwrap_or_default();
        let m = merchants.get(shop_id.as_str());
        let delivery = o.get("deliveryTime").and_then(|d| Some((d.get("minDays")?.as_u64()?, d.get("maxDays")?.as_u64()?)));
        offers.push(Offer {
            id,
            shop: m.and_then(|m| text(m, "name")).unwrap_or_else(|| shop_id.clone()),
            shop_page: m.and_then(|m| text(m, "url")).map(|u| if u.starts_with('/') { format!("{}{u}", site.origin()) } else { u }),
            shop_logo: m.and_then(|m| m.get("logo")).and_then(|l| text(l, "url")),
            shop_id,
            price,
            currency: o.get("price").and_then(|p| text(p, "currency")).unwrap_or_else(|| site.currency.into()),
            shipping: o.get("shippingCost").filter(|s| s.is_object()).and_then(amount),
            stock: text(o, "stockStatus").unwrap_or_else(|| "UNKNOWN".into()),
            url: text(o, "url").unwrap_or_default(),
            delivery_days: delivery,
        });
    }
    offers.sort_by(|a, b| a.price.total_cmp(&b.price).then_with(|| a.shop.cmp(&b.shop)));

    let national = v.get("offersSummary").and_then(|s| s.get("nationalOffer")).filter(|n| n.is_object());
    let count = national.and_then(|n| n.get("count")).and_then(Value::as_u64).unwrap_or(offers.len() as u64);
    let highest = national.and_then(|n| n.get("maxPrice")).filter(|p| p.is_object()).and_then(amount);
    let picture = v.get("images").and_then(Value::as_array).and_then(|i| i.first()).and_then(|i| text(i, "url").or_else(|| text(i, "path")));
    Prices { offers, count, highest, picture: picture.as_deref().map(picture_url) }
}

impl Prices {
    pub fn cheapest(&self) -> Option<&Offer> {
        self.offers.first()
    }

    /// The lowest price in each shop, cheapest shop first.
    pub fn by_shop(&self) -> Map<String, Value> {
        let mut m = Map::new();
        for o in &self.offers {
            if !m.contains_key(&o.shop) {
                m.insert(o.shop.clone(), json!(o.price));
            }
        }
        m
    }
}

/// A price as a state: `6118` or `7569.35`.
pub fn price_state(p: f64) -> String {
    let s = format!("{p:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
