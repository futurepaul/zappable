mod config;

use std::{str::FromStr, time::Duration};

use config::Config;
use nostr_sdk::{
    prelude::{FromBech32, FromSkStr},
    secp256k1::XOnlyPublicKey,
    Client, EventBuilder, EventId, Filter, Keys, Kind, Metadata, Tag, Url,
};
use serde::{Deserialize, Serialize};
use std::net::ToSocketAddrs;
use tonic_lnd::{lnrpc::SendRequest, LightningClient};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LnurlPayResponse {
    pub callback: String,
    pub max_sendable: u64,
    pub min_sendable: u64,
    pub metadata: String,
    pub tag: String,
    pub allows_nostr: bool,
    pub nostr_pubkey: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LnurlPayCallbackResponse {
    pub pr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load the configuration file
    let config = Config::new();

    let event_id = std::env::args().nth(2).expect("no event id provided");
    let event_id = EventId::from_bech32(&event_id).unwrap();

    let keys = Keys::from_sk_str(&config.nsec).expect("invalid nsec key");

    let client = Client::new(&keys);

    for relay in config.relays.clone() {
        client.add_relay(&relay, None).await?;
    }

    client.connect().await;

    // Get first event that matches the event id
    let event_filter = Filter::new().id(event_id).limit(1);

    let timeout = Duration::from_secs(10);

    let events = client
        .get_events_of(vec![event_filter], Some(timeout))
        .await?;

    let event = events.first().expect("no events found with that note id");

    dbg!(event.clone());

    // Get lightning address from user pubkey
    let metadata_filter = Filter::new()
        .author(event.pubkey)
        .kind(Kind::Metadata)
        .limit(1);

    let metadatas = client
        .get_events_of(vec![metadata_filter], Some(timeout))
        .await?;

    dbg!(metadatas.clone());

    let metadata_json = metadatas
        .first()
        .expect("no metadata found for that user")
        .clone()
        .content;

    let lightning_address =
        if let Some(ln_address) = Metadata::from_json(&metadata_json).unwrap().lud16 {
            ln_address
        } else {
            panic!("no lightning address found for that user");
        };

    dbg!(lightning_address.clone());

    // Make the lnurl request

    // Split the lightning address into host and name on the @ symbol
    let (name, host) = lightning_address
        .split_once('@')
        .expect("invalid lightning address");

    // If running locally, use http, otherwise use https
    let lnurl_address = if host.contains("localhost") {
        format!("http://{host}/.well-known/lnurlp/{name}")
    } else {
        format!("https://{host}/.well-known/lnurlp/{name}")
    };

    // Get the lnurl response using reqwest
    let lnurl_response = reqwest::get(&lnurl_address)
        .await?
        .json::<LnurlPayResponse>()
        .await?;

    dbg!(lnurl_response.clone());

    if !lnurl_response.clone().allows_nostr {
        panic!("zaps aren't enabled for this lightning address");
    }

    let nostr_pubkey = lnurl_response
        .nostr_pubkey
        .expect("no nostr pubkey found in lnurl response");

    let pubkey = XOnlyPublicKey::from_str(&nostr_pubkey).unwrap();

    let e = Tag::Event(event_id, None, None);
    let p = Tag::PubKey(pubkey, None);

    // Iterate over vec of strings and convert to vec of urls, fail if any fail
    let relays_as_urls = config
        .relays
        .iter()
        .map(|relay| Url::from_str(relay))
        .collect::<Result<Vec<Url>, _>>()?;

    let relays = Tag::Relays(relays_as_urls);

    let zap_request = EventBuilder::new(Kind::ZapRequest, "", &[e, p, relays]).to_event(&keys)?;
    let zap_request_json = zap_request.as_json();

    // Hit the lnurlp callback with the amount and the zap request
    let lnurl_callback_url = lnurl_response.callback;

    let uri_encoded_zap_request = urlencoding::encode(&zap_request_json);

    let zap_request_url = format!(
        "{}&amount={}&nostr={}",
        lnurl_callback_url, config.amount_sats, uri_encoded_zap_request
    );

    // dbg!(zap_request_url);

    let callback_response = reqwest::get(&zap_request_url)
        .await?
        .json::<LnurlPayCallbackResponse>()
        .await?;

    dbg!(callback_response.clone());

    // Pay the invoice using lnd from the config

    let server: Vec<_> = config
        .node_grpc_host
        .to_socket_addrs()
        .expect("Unable to resolve domain")
        .collect();

    let mut lnd_client: LightningClient = match tonic_lnd::connect(
        server[0].ip().to_string(),
        server[0].port() as u32,
        config.node_tls_path,
        config.node_macaroon_path,
    )
    .await
    {
        Ok(mut ln) => ln.lightning().clone(),
        Err(_) => panic!("could not connect"),
    };

    // JUST IN CASE YOU WANT TO DEBUG YOUR LND CONNECTION
    // let info = lnd_client.get_info(GetInfoRequest {}).await?;

    let _ = lnd_client
        .send_payment_sync(SendRequest {
            payment_request: callback_response.pr,
            ..Default::default()
        })
        .await?;

    // Find the zap events that the receiver ("zapper" as they're confusingly called) published
    let zap_filter = Filter::new().kind(Kind::Zap).event(event_id).pubkey(pubkey);

    let zaps = client
        .get_events_of(vec![zap_filter], Some(timeout))
        .await?;

    dbg!(zaps);

    Ok(())
}
