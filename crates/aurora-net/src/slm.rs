//! The Second Life Marketplace (SLM) API of the "Marketplace listings"
//! window, as LLMarketplaceData (indra/newview/llmarketplacefunctions.cpp,
//! originally LGPL 2.1): the requests on the region's `DirectDelivery`
//! capability, their JSON bodies, and the reading of the answers. Pure: the
//! session sends them (`session.rs`), the world keeps the listings
//! (`aurora-viewer` `world/marketplace.rs`).

use serde_json::{Value, json};
use uuid::Uuid;

/// One listing of the merchant, as the SLM API describes it
/// (`listings[]` of every answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub id: i32,
    pub is_listed: bool,
    pub edit_url: String,
    /// `inventory_info.listing_folder_id`: the folder of the listing under
    /// "Marketplace listings".
    pub folder: Uuid,
    /// `inventory_info.version_folder_id`: the folder that is sold (nil when
    /// none was chosen).
    pub version: Uuid,
    /// `inventory_info.count_on_hand`: -1 for an unlimited stock.
    pub count: i32,
}

/// What is asked of the SLM API (getSLMListingsCoro, getSingleListingCoro,
/// createSLMListingCoro, updateSLMListingCoro, associateSLMListingCoro,
/// deleteSLMListingCoro).
#[derive(Debug, Clone, PartialEq)]
pub enum SlmRequest {
    /// GET /listings: every listing of the merchant.
    Listings,
    /// GET /listing/<id>.
    Listing { id: i32 },
    /// POST /listings: a listing for `folder`, named after it.
    Create {
        folder: Uuid,
        name: String,
        version: Uuid,
        count: i32,
    },
    /// PUT /listing/<id>: version, listed state and stock.
    Update {
        folder: Uuid,
        id: i32,
        version: Uuid,
        is_listed: bool,
        count: i32,
    },
    /// PUT /associate_inventory/<id>: an existing listing on another folder
    /// (`source` is the folder the drop came from, refreshed afterwards).
    Associate {
        folder: Uuid,
        id: i32,
        version: Uuid,
        source: Uuid,
    },
    /// DELETE /listing/<id>.
    Delete { id: i32 },
}

/// The HTTP method of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl SlmRequest {
    /// "Get /listings"…, the name the warnings of log_SLM_warning give.
    pub fn name(&self) -> &'static str {
        match self {
            SlmRequest::Listings => "Get /listings",
            SlmRequest::Listing { .. } => "Get /listing",
            SlmRequest::Create { .. } => "Post /listings",
            SlmRequest::Update { .. } => "Put /listing",
            SlmRequest::Associate { .. } => "Put /associate_inventory",
            SlmRequest::Delete { .. } => "Delete /listing",
        }
    }

    pub fn method(&self) -> Method {
        match self {
            SlmRequest::Listings | SlmRequest::Listing { .. } => Method::Get,
            SlmRequest::Create { .. } => Method::Post,
            SlmRequest::Update { .. } | SlmRequest::Associate { .. } => Method::Put,
            SlmRequest::Delete { .. } => Method::Delete,
        }
    }

    /// getSLMConnectURL: the `DirectDelivery` capability followed by the
    /// route (the same join as the existing `/merchant` request).
    pub fn url(&self, cap: &str) -> String {
        let route = match self {
            SlmRequest::Listings | SlmRequest::Create { .. } => "/listings".to_owned(),
            SlmRequest::Listing { id } | SlmRequest::Update { id, .. } | SlmRequest::Delete { id } => format!("/listing/{id}"),
            SlmRequest::Associate { id, .. } => format!("/associate_inventory/{id}"),
        };
        format!("{}{route}", cap.trim_end_matches('/'))
    }

    /// The JSON body (`postData`), None for GET and DELETE.
    pub fn body(&self) -> Option<Value> {
        match self {
            SlmRequest::Listings | SlmRequest::Listing { .. } | SlmRequest::Delete { .. } => None,
            SlmRequest::Create {
                folder,
                name,
                version,
                count,
            } => Some(json!({
                "listing": {
                    "name": name,
                    "inventory_info": {
                        "listing_folder_id": folder.to_string(),
                        "version_folder_id": version.to_string(),
                        "count_on_hand": count,
                    },
                }
            })),
            SlmRequest::Update {
                folder,
                id,
                version,
                is_listed,
                count,
            } => Some(json!({
                "listing": {
                    "inventory_info": {
                        "listing_folder_id": folder.to_string(),
                        "version_folder_id": version.to_string(),
                        "count_on_hand": count,
                    },
                    "id": id,
                    "is_listed": is_listed,
                }
            })),
            SlmRequest::Associate { folder, id, version, .. } => Some(json!({
                "listing": {
                    "id": id,
                    "inventory_info": {
                        "listing_folder_id": folder.to_string(),
                        "version_folder_id": version.to_string(),
                    },
                }
            })),
        }
    }

    /// The folders shown as updating while the request runs (setUpdating).
    pub fn folders(&self) -> Vec<Uuid> {
        match self {
            SlmRequest::Listings | SlmRequest::Listing { .. } | SlmRequest::Delete { .. } => Vec::new(),
            SlmRequest::Create { folder, .. } | SlmRequest::Update { folder, .. } => vec![*folder],
            SlmRequest::Associate { folder, source, .. } => vec![*folder, *source],
        }
    }
}

/// How an SLM request ended, read from its status and body.
#[derive(Debug, Clone, PartialEq)]
pub enum SlmReply {
    /// A success: the listings of the answer (possibly none).
    Listings(Vec<Listing>),
    /// GET /listing/<id> answered 404: that listing no longer exists.
    NotFound,
    /// A failure, with what MerchantTransactionFailed shows.
    Failed(SlmError),
}

/// The alert of log_SLM_warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlmError {
    /// 422 with more than four lines of content: the frequent answer to
    /// listing an incomplete listing (MerchantUnprocessableEntity).
    Unprocessable,
    /// MerchantTransactionFailed: `[ERROR_REASON]` (empty when the reason is
    /// "unknown" and a description exists) and `[ERROR_DESCRIPTION]`.
    Transaction { reason: String, description: String },
}

/// The merchant status of getMerchantStatusCoro (MarketplaceStatusCodes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MerchantStatus {
    Merchant,
    /// 404: the account has no Marketplace store.
    NotMerchant,
    /// 503: a merchant of the old Marketplace that was never migrated.
    NotMigrated,
    /// Any other failure, or no capability: the reason is written in the
    /// window rather than shown in an alert.
    ConnectionFailure(String),
}

/// getMerchantStatusCoro: the status from the HTTP code of GET /merchant.
pub fn merchant_status(code: u16, reason: &str) -> MerchantStatus {
    match code {
        200..=299 => MerchantStatus::Merchant,
        404 => MerchantStatus::NotMerchant,
        503 => MerchantStatus::NotMigrated,
        _ => MerchantStatus::ConnectionFailure(reason.to_owned()),
    }
}

/// The `listings[]` of an answer. A listing without a folder is skipped,
/// like getSLMListingsCoro does (`folderUuid.notNull()`); an unreadable
/// field reads as LLSD's default (0, false, empty, nil).
pub fn parse_listings(body: &Value) -> Vec<Listing> {
    let Some(list) = body.get("listings").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|l| {
            let info = &l["inventory_info"];
            Listing {
                id: as_int(&l["id"]),
                is_listed: as_bool(&l["is_listed"]),
                edit_url: l["edit_url"].as_str().unwrap_or("").to_owned(),
                folder: as_uuid(&info["listing_folder_id"]),
                version: as_uuid(&info["version_folder_id"]),
                count: as_int(&info["count_on_hand"]),
            }
        })
        .filter(|l| !l.folder.is_nil())
        .collect()
}

/// The answer of `request` from its HTTP code, status text and body.
pub fn reply(request: &SlmRequest, code: u16, reason: &str, body: &Value) -> SlmReply {
    if (200..300).contains(&code) {
        return SlmReply::Listings(parse_listings(body));
    }
    if code == 404 && matches!(request, SlmRequest::Listing { .. }) {
        return SlmReply::NotFound;
    }
    SlmReply::Failed(error(code, reason, body))
}

/// log_SLM_warning: what the alert says about a failed request.
pub fn error(code: u16, reason: &str, body: &Value) -> SlmError {
    if code == 422 && body.as_array().is_some_and(|a| a.len() > 4) {
        return SlmError::Unprocessable;
    }
    // an array of lines is joined; a map reads as LLSD's asString of a map,
    // which is empty
    let description = match body {
        Value::Array(lines) => lines.iter().map(as_string).collect::<Vec<_>>().join("\n"),
        Value::Null | Value::Object(_) => String::new(),
        other => as_string(other),
    };
    let reason = if !description.is_empty() && reason.to_lowercase().contains("unknown") {
        String::new()
    } else {
        format!("'{reason}'\n")
    };
    SlmError::Transaction { reason, description }
}

fn as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// LLSD::asInteger of a JSON value: numbers, numeric strings, booleans.
fn as_int(v: &Value) -> i32 {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0) as i32,
        Value::String(s) => s.trim().parse().unwrap_or(0),
        Value::Bool(b) => *b as i32,
        _ => 0,
    }
}

fn as_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => matches!(s.as_str(), "true" | "1"),
        _ => false,
    }
}

fn as_uuid(v: &Value) -> Uuid {
    v.as_str().and_then(|s| Uuid::parse_str(s).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn routes_follow_the_direct_delivery_capability() {
        let cap = "https://cap.example/slm/abc/";
        assert_eq!(SlmRequest::Listings.url(cap), "https://cap.example/slm/abc/listings");
        assert_eq!(SlmRequest::Listing { id: 12 }.url(cap), "https://cap.example/slm/abc/listing/12");
        assert_eq!(SlmRequest::Delete { id: 12 }.url(cap), "https://cap.example/slm/abc/listing/12");
        let associate = SlmRequest::Associate {
            folder: id(1),
            id: 7,
            version: id(2),
            source: id(3),
        };
        assert_eq!(associate.url(cap), "https://cap.example/slm/abc/associate_inventory/7");
        assert_eq!(associate.method(), Method::Put);
        assert_eq!(associate.folders(), vec![id(1), id(3)]);
        assert_eq!(SlmRequest::Delete { id: 1 }.method(), Method::Delete);
    }

    #[test]
    fn bodies_match_firestorm() {
        let create = SlmRequest::Create {
            folder: id(1),
            name: "Hoodie".into(),
            version: id(2),
            count: 5,
        };
        assert_eq!(
            create.body().unwrap(),
            json!({"listing": {"name": "Hoodie", "inventory_info": {
                "listing_folder_id": id(1).to_string(), "version_folder_id": id(2).to_string(), "count_on_hand": 5}}})
        );
        let update = SlmRequest::Update {
            folder: id(1),
            id: 9,
            version: id(2),
            is_listed: true,
            count: -1,
        };
        assert_eq!(
            update.body().unwrap(),
            json!({"listing": {"id": 9, "is_listed": true, "inventory_info": {
                "listing_folder_id": id(1).to_string(), "version_folder_id": id(2).to_string(), "count_on_hand": -1}}})
        );
        let associate = SlmRequest::Associate {
            folder: id(1),
            id: 9,
            version: id(2),
            source: id(3),
        };
        assert_eq!(
            associate.body().unwrap(),
            json!({"listing": {"id": 9, "inventory_info": {
                "listing_folder_id": id(1).to_string(), "version_folder_id": id(2).to_string()}}})
        );
        assert!(SlmRequest::Listings.body().is_none());
        assert!(SlmRequest::Delete { id: 9 }.body().is_none());
    }

    #[test]
    fn listings_are_read_like_llsd_and_folderless_ones_skipped() {
        let body = json!({"listings": [
            {"id": 4, "is_listed": true, "edit_url": "https://m/e/4",
             "inventory_info": {"listing_folder_id": id(1).to_string(), "version_folder_id": id(2).to_string(), "count_on_hand": 3}},
            {"id": "5", "is_listed": 0, "inventory_info": {"listing_folder_id": id(6).to_string(), "count_on_hand": -1}},
            {"id": 8, "is_listed": true, "inventory_info": {}}
        ]});
        let l = parse_listings(&body);
        assert_eq!(l.len(), 2);
        assert_eq!(
            l[0],
            Listing {
                id: 4,
                is_listed: true,
                edit_url: "https://m/e/4".into(),
                folder: id(1),
                version: id(2),
                count: 3
            }
        );
        assert_eq!((l[1].id, l[1].is_listed, l[1].version, l[1].count), (5, false, Uuid::nil(), -1));
        assert!(parse_listings(&json!({})).is_empty());
    }

    #[test]
    fn failures_become_the_alerts_of_firestorm() {
        let update = SlmRequest::Update {
            folder: id(1),
            id: 9,
            version: id(2),
            is_listed: true,
            count: 0,
        };
        // an incomplete listing: 422 with a long list of reasons
        let long = json!(["a", "b", "c", "d", "e"]);
        assert_eq!(
            reply(&update, 422, "Unprocessable Entity", &long),
            SlmReply::Failed(SlmError::Unprocessable)
        );
        // a short 422 is an ordinary failure, its lines joined
        assert_eq!(
            error(422, "Unprocessable Entity", &json!(["no version", "no price"])),
            SlmError::Transaction {
                reason: "'Unprocessable Entity'\n".into(),
                description: "no version\nno price".into()
            }
        );
        // an "unknown" reason is left out when a description exists
        assert_eq!(
            error(500, "Unknown error", &json!("server down")),
            SlmError::Transaction {
                reason: String::new(),
                description: "server down".into()
            }
        );
        // a missing single listing is not an error but a removal
        assert_eq!(
            reply(&SlmRequest::Listing { id: 3 }, 404, "Not Found", &Value::Null),
            SlmReply::NotFound
        );
        assert!(matches!(
            reply(&SlmRequest::Delete { id: 3 }, 404, "Not Found", &Value::Null),
            SlmReply::Failed(_)
        ));
        assert_eq!(
            reply(&SlmRequest::Listings, 200, "OK", &json!({"listings": []})),
            SlmReply::Listings(Vec::new())
        );
    }

    #[test]
    fn merchant_codes() {
        assert_eq!(merchant_status(200, "OK"), MerchantStatus::Merchant);
        assert_eq!(merchant_status(404, "Not Found"), MerchantStatus::NotMerchant);
        assert_eq!(merchant_status(503, "Service Unavailable"), MerchantStatus::NotMigrated);
        assert_eq!(merchant_status(500, "boom"), MerchantStatus::ConnectionFailure("boom".into()));
    }
}
