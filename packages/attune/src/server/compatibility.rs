use axum::{
    Json,
    http::{HeaderMap, StatusCode},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::api::ErrorResponse;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CompatibilityResponse {
    Ok,
    WarnUpgrade { latest: String },
    Incompatible { minimum: String },
}

pub const API_VERSION_HEADER: &str = "X-API-Version";

pub const API_VERSION_HEADER_V0_2_0: &str = "2025-07-24";

// TODO: Should this be a layer instead? If we make it into a layer, we could
// return an `X-Upgrade-To` header on "warning" and return a 500 on
// "incompatible".
//
// How would we handle the warning case on the client side? Is there a way to
// add "default layers" to reqwest's response handling for a specific client? Or
// should we just write our own client?
#[axum::debug_handler]
pub async fn handler(headers: HeaderMap) -> Result<Json<CompatibilityResponse>, ErrorResponse> {
    let version = match headers.get(API_VERSION_HEADER) {
        Some(version) => match version.to_str() {
            Ok(version) => version,
            Err(err) => {
                return Err(ErrorResponse::new(
                    StatusCode::BAD_REQUEST,
                    "API_VERSION_HEADER_INVALID".to_string(),
                    format!("API version header invalid: {}", err),
                ));
            }
        },
        None => {
            return Err(ErrorResponse::new(
                StatusCode::BAD_REQUEST,
                "API_VERSION_HEADER_MISSING".to_string(),
                "API version header missing".to_string(),
            ));
        }
    };
    let version_date = match NaiveDate::parse_from_str(version, "%Y-%m-%d") {
        Ok(version_date) => version_date,
        Err(err) => {
            return Err(ErrorResponse::new(
                StatusCode::BAD_REQUEST,
                "API_VERSION_HEADER_INVALID".to_string(),
                format!("could not parse API version header: {}", err),
            ));
        }
    };

    if version_date < NaiveDate::parse_from_str(API_VERSION_HEADER_V0_2_0, "%Y-%m-%d").unwrap() {
        return Ok(Json(CompatibilityResponse::Incompatible {
            minimum: API_VERSION_HEADER_V0_2_0.to_string(),
        }));
    }
    Ok(Json(CompatibilityResponse::Ok))
}
