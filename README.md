# Pricerunner for HomewrightHQ

An external integration for [HomewrightHQ](https://github.com/magnusdunker/homewrighthq) that keeps an eye on the prices of products on [Pricerunner](https://www.pricerunner.dk). It shows the lowest price, the shop that has it, and what the other shops ask. It needs no key or account.

Pricerunner has no official public API. The integration reads the same JSON that Pricerunner's own product pages fetch. That is not documented and can change without notice, and then the integration stops working until it is updated. It is not made by or connected to Pricerunner or Klarna.

## Installation

In HomewrightHQ: **Settings → Integrations → From GitHub**. Paste

```
https://github.com/magnusdunker/homewrighthq-pricerunner
```

and press **Install**. Then add **Pricerunner** under **Add integration**.

Pick the country, then paste links to the products, one per line. A link is the address of the product's page on Pricerunner, e.g.

```
https://www.pricerunner.dk/pl/1-3397102161/Mobiltelefoner/Apple-iPhone-16-128GB-Black-Sammenlign-Priser
```

A product id such as `3397102161` works too. You can also type a name under **Søg** (search) and pick from the products Pricerunner finds. Products can be removed and added later under the integration's options.

The countries are Denmark (pricerunner.dk, DKK), Sweden (pricerunner.se, SEK) and the United Kingdom (pricerunner.com, GBP). Each entry has one country; add the integration again for another one.

## What you get

A device per product with two entities:

- **Laveste pris** (lowest price, `sensor`, device class `monetary`, unit DKK, SEK or GBP): the lowest price in the country, as on the product page. The attributes are:
  - `shop`: the shop with the lowest price
  - `shop_url`: the link to the offer in the shop, through Pricerunner
  - `shop_page`: the shop's page on Pricerunner
  - `stock`: `in_stock`, `backorder`, `preorder`, `special_order`, `collect_in_store` or `unknown`
  - `shipping` and `price_with_shipping`, when the shop states the shipping cost
  - `delivery_days`: e.g. "2-4"
  - `highest_price`: the highest price in the country
  - `offers`: the number of offers, as Pricerunner counts them
  - `shops` and `shop_prices`: the number of shops, and each shop's lowest price, cheapest first, e.g. `{"NordicElectronics": 6118.0, "INphone.dk": 6169.0, ...}`
  - `cheapest`: the five cheapest offers with `shop`, `price`, `shipping`, `stock` and `url`
  - `product`, `product_id` and `product_url`: the product's name, id and page on Pricerunner
  - `entity_picture`: the product's picture
  - `currency`
- **Billigste butik** (cheapest shop, `sensor`): the name of the shop with the lowest price, with `price`, `currency`, `shop_url`, `shop_page`, `product_url` and the shop's logo as the picture.

The prices count new products from shops in the country that do not have them sold out, the same filter the product page starts with. Sponsored offers are included. If no shop has the product, both entities are `unknown`.

The prices are fetched every other hour, with two seconds between products. If that fails three times in a row, the product's entities become unavailable until it succeeds again. A product that Pricerunner removes becomes unavailable the same way.

There is no entity for the price in one particular shop. `shop_prices` has it:

```yaml
{{ state_attr('sensor.apple_iphone_16_128gb_black_laveste_pris', 'shop_prices')['Elgiganten'] }}
```

A message when the price drops below 6,000 kr.:

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

The integration asks the site's own API under `https://www.pricerunner.dk/dk/api/` (`/se/api/` on pricerunner.se and `/uk/api/` on pricerunner.com):

- `product-detail-edge-rest/public/product-detail/v0/initial/<country>/<category>/<id>`: the product's name, brand and category, when you add it. The category is the first number in the link (`/pl/1-3397102161` is category 1); `0` works when it is not known.
- `product-detail-edge-rest/public/product-detail/v0/offers/<country>/<id>`: the offers, shops and pictures, at each update. It asks with `af_ORIGIN=NATIONAL`, `af_ITEM_CONDITION=NEW,UNKNOWN`, the stock statuses above and `sortByPreset=PRICE`. An unknown id is answered with 204 No Content.
- `instant-search-edge-rest/public/search/suggest/<country>?q=...`: the search, which gives the first five products.

The integration sends its own `User-Agent` (`homewright-pricerunner/<version>`). Pricerunner answers 403 to curl's default one.

Germany is not included: pricerunner.de now sends its API to klarna.com.

## Not tested

- The integration has been run against the real Pricerunner in Denmark, Sweden and the United Kingdom with `cargo test -- --ignored live`, through a fake HomewrightHQ. It has not been installed in a running HomewrightHQ yet, so the forms have not been seen in the web interface.
- How long Pricerunner accepts requests from the same address is not known. Polling every other hour with a pause between products should keep well under any limit, but there is no documented one.
- Products with several variants and products sold by the unit (`pricePerUnit`) have not been looked at closely; the price used is the offer's `price`.

## Development

Built in Rust on the `thehome-plugin` SDK from [homewright-example-integration-rust](https://github.com/magnusdunker/homewrighthq-example-integration-rust).

- `src/api.rs`: the sites, the links and the JSON from Pricerunner
- `src/lib.rs`: the flows, the setup, the update and the connection to HomewrightHQ
- `tests/`: tests against real responses from Pricerunner in `tests/fixtures` (fetched 2026-11-02 and trimmed: fewer offers, merchants and filters, no product texts), against a fake Pricerunner and with a fake HomewrightHQ in `tests/host`

```
cargo test
cargo test -- --ignored live --nocapture   # against Pricerunner
cargo build --release
```

A new version is released with a tag, e.g. `git tag v0.2.0 && git push origin v0.2.0`. The workflow builds for Windows, Linux (x86_64 and arm64, static musl) and macOS and puts the binaries, `homewright.json` and the translations on the release.

## License

MIT. Prices, product names and pictures are from Pricerunner.
