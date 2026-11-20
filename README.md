# Pricerunner for HomewrightHQ

An external integration for [HomewrightHQ](https://github.com/magnusdunker/homewrighthq) that keeps an eye on the prices of products on [Pricerunner](https://www.pricerunner.dk), and on the same price comparison on [Klarna](https://www.klarna.com/de/shopping/) in other countries. It shows the lowest price, the shop that has it, and what the other shops ask. It needs no key or account.

Pricerunner has no official public API. The integration reads the same JSON that Pricerunner's own product pages fetch. That is not documented and can change without notice, and then the integration stops working until it is updated. It is not made by or connected to Pricerunner or Klarna.

## Installation

In HomewrightHQ: **Settings → Integrations → From GitHub**. Paste

```
https://github.com/magnusdunker/homewrighthq-pricerunner
```

and press **Install**. Then add **Pricerunner** under **Add integration**.

Pick one or more countries, then paste links to the products, one per line. A link is the address of the product's page on Pricerunner or Klarna, e.g.

```
https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser
https://www.klarna.com/de/shopping/pl/cl94/3216399795/Kopfhoerer/Apple-AirPods-Pro-3/
```

A product id such as `3397102161` works too. The ids are the same in every country, so a product found in Denmark is followed in Germany as well. You can also type a name under **Søg** (search) and pick from the products found in the first country you picked. Products and countries can be changed later under the integration's options.

The countries are:

- on Pricerunner: Denmark (pricerunner.dk, DKK), Sweden (pricerunner.se, SEK) and the United Kingdom (pricerunner.com, GBP)
- on Klarna: Norway (NOK), Finland, Germany, Austria, Switzerland (CHF), the Netherlands, Belgium, France, Spain, Italy, Poland (PLN), Ireland and the USA (USD). The others are in EUR. pricerunner.de, .fr and .at send you to these pages.

**Medtag udenlandske butikker** (include foreign shops) also counts shops abroad that sell to the country. On pricerunner.dk that is e.g. Amazon.de, with its price in DKK. It is off by default, as on the product page.

## What you get

A device per product with two entities per country. With several countries the entity names end with the country, e.g. **Laveste pris Tyskland**.

- **Laveste pris** (lowest price, `sensor`, device class `monetary`, the country's currency as unit): the lowest price in the country, as on the product page. The attributes are:
  - `shop`: the shop with the lowest price
  - `shop_url`: the link to the offer in the shop, through Pricerunner or Klarna
  - `shop_page`: the shop's page on Pricerunner. Klarna has no shop pages, so it is empty there
  - `stock`: `in_stock`, `backorder`, `preorder`, `special_order`, `collect_in_store` or `unknown`
  - `shipping` and `price_with_shipping`, when the shop states the shipping cost
  - `delivery_days`: e.g. "2-4"
  - `highest_price`: the highest price in the country
  - `offers`: the number of offers, as Pricerunner counts them, with foreign shops when they are included
  - `shops` and `shop_prices`: the number of shops, and each shop's lowest price, cheapest first, e.g. `{"NordicElectronics": 6118.0, "INphone.dk": 6169.0, ...}`
  - `cheapest`: the five cheapest offers with `shop`, `price`, `shipping`, `stock` and `url`
  - `product`, `product_id` and `product_url`: the product's name, id and page in the country
  - `entity_picture`: the product's picture
  - `currency`
  - `country`: the country's key, e.g. `dk` or `de`
- **Billigste butik** (cheapest shop, `sensor`): the name of the shop with the lowest price, with `price`, `currency`, `shop_url`, `shop_page`, `product_url` and the shop's logo as the picture.

The prices count new products from shops in the country that do not have them sold out, the same filter the product page starts with, and shops abroad when foreign shops are included. Sponsored offers are included. If no shop has the product, both entities are `unknown`.

The prices are fetched every other hour, with two seconds between each product in each country. If that fails three times in a row, the product's entities become unavailable until it succeeds again. A product that Pricerunner removes becomes unavailable the same way.

There is no entity for the price in one particular shop. `shop_prices` has it:

```yaml
{{ state_attr('sensor.apple_iphone_16_128gb_black_laveste_pris', 'shop_prices')['Elgiganten'] }}
```

## Price alerts

After the products, the integration asks for a price alert: a maximum price per product and country, in the country's currency, and where the message goes. The same step comes after the products under the integration's options, so the alerts can be changed later. A field left empty gets no message.

When the lowest price is at or below the maximum, one message is sent, e.g. "Apple iPhone 16, 128GB Black koster 6118 DKK hos NordicElectronics. Din grænse er 6500 DKK." With several countries the message names the country: "... hos Amazon i Tyskland." It can go as:

- **Push-besked** (push notification): `notify.web_push` to the phones and browsers with push turned on. Pick users under **Push til brugere** to send only to their devices; with none picked, the message goes to everyone with push turned on. The user list needs HomewrightHQ 0.5.47 or newer. Tapping the message opens the product's page in the country.
- **Notifikation i brugerfladen** (notification in the interface): `persistent_notification.create`, with a link to the product's page.
- **Alle beskedtjenester** (all message services): `notify.notify`.
- **En anden tjeneste** (another service): a `notify` service you type in, e.g. `notify.telegram`.

A message is sent once. The integration remembers it in `alerts.json` in its data folder, so updates and restarts do not send it again. When the price goes above the maximum, or the maximum is changed, the next price at or below it sends a new message. A product sold out everywhere changes nothing.

For other conditions, an automation on the price sensor works too. A message when the price drops below 6,000 kr.:

```yaml
triggers:
  - trigger: numeric_state
    entity_id: sensor.apple_iphone_16_128gb_black_laveste_pris
    below: 6000
actions:
  - action: notify.notify
    data:
      message: >
        {{ state_attr(trigger.entity_id, 'product') }} costs
        {{ states(trigger.entity_id) }} kr. at {{ state_attr(trigger.entity_id, 'shop') }}.
```

## Where the prices come from

The integration asks the site's own API under `https://www.pricerunner.dk/dk/api/` (`/se/api/` on pricerunner.se and `/uk/api/` on pricerunner.com). Klarna has the same API under the country, e.g. `https://www.klarna.com/de/api/`, and its product pages are under `https://www.klarna.com/de/shopping/`:

- `product-detail-edge-rest/public/product-detail/v0/initial/<country>/<category>/<id>`: the product's name, brand and category, when you add it. The category is the first number in the link (`/pl/1-3397102161` and `/pl/cl1/3397102161` are category 1); `0` works when it is not known.
- `product-detail-edge-rest/public/product-detail/v0/offers/<country>/<id>`: the offers, shops and pictures, at each update. It asks with `af_ORIGIN=NATIONAL` (left out when foreign shops are included), `af_ITEM_CONDITION=NEW,UNKNOWN`, the stock statuses above and `sortByPreset=PRICE`. An unknown id is answered with 204 No Content.
- `instant-search-edge-rest/public/search/suggest/<country>?q=...`: the search, which gives the first five products.

The integration sends its own `User-Agent` (`homewright-pricerunner/<version>`). Pricerunner answers 403 to curl's default one.

Canada and Australia are not included: on 2026-11-20 Klarna's Canadian site had no offers and the Australian one answered 404.

## Not tested

- The integration has been run against the real Pricerunner in Denmark, Sweden and the United Kingdom, and Klarna in Germany and the USA, with `cargo test -- --ignored live`, through a fake HomewrightHQ. The other Klarna countries answered the search and the offers on 2026-11-20 but have not been through the integration. It has not been installed in a running HomewrightHQ yet, so the forms have not been seen in the web interface.
- How long Pricerunner accepts requests from the same address is not known. Polling every other hour with a pause between products should keep well under any limit, but there is no documented one.
- Klarna can list an offer that is not the product alone. On 2026-11-20 the lowest German price for AirPods Pro 3 was for a while 24 EUR, from congstar with a mobile plan. An hour later the offers were gone. Nothing in the offer marks it as a bundle, so the integration cannot leave it out, and such a price can set off a price alert.
- Products with several variants and products sold by the unit (`pricePerUnit`) have not been looked at closely; the price used is the offer's `price`.

## Development

Built in Rust on the `thehome-plugin` SDK from [homewright-example-integration-rust](https://github.com/magnusdunker/homewrighthq-example-integration-rust).

- `src/api.rs`: the sites, the links and the JSON from Pricerunner and Klarna
- `src/lib.rs`: the flows, the setup, the update and the connection to HomewrightHQ
- `src/alerts.rs`: the price alerts: the form, the settings and the messages
- `tests/`: tests against real responses from Pricerunner in `tests/fixtures` (fetched 2026-11-02 and trimmed: fewer offers, merchants and filters, no product texts), against a fake Pricerunner and with a fake HomewrightHQ in `tests/host`

```
cargo test
cargo test -- --ignored live --nocapture   # against Pricerunner
cargo build --release
```

A new version is released with a tag, e.g. `git tag v0.2.0 && git push origin v0.2.0`. The workflow builds for Windows, Linux (x86_64 and arm64, static musl) and macOS and puts the binaries, `homewright.json` and the translations on the release.

## License

MIT. Prices, product names and pictures are from Pricerunner and Klarna.
