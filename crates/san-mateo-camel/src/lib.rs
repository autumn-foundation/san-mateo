//! A rust-camel component for San Mateo.
//!
//! Camel routes call provider APIs with the auth that San Mateo keeps. San
//! Mateo refreshes `OAuth2` tokens before each call when they expire soon. It
//! also refreshes one time after a 401 reply.
//!
//! The component is producer-only. Register it with a [`SanMateo`] service.
//! Then send exchanges to a `san-mateo:<integration>` endpoint:
//!
//! ```no_run
//! # async fn demo(san: san_mateo::SanMateo) -> Result<(), camel_api::CamelError> {
//! use camel_builder::{RouteBuilder, StepAccumulator};
//! use camel_component_direct::DirectComponent;
//! use camel_core::CamelContext;
//! use san_mateo_camel::SanMateoComponent;
//!
//! let mut camel = CamelContext::builder().build().await?;
//! camel.register_component(DirectComponent::new());
//! camel.register_component(SanMateoComponent::new(san).environment("prod"));
//! let route = RouteBuilder::from("direct:contacts")
//!     .route_id("list-contacts")
//!     .to("san-mateo:hubspot?connectionId=c1&path=/crm/v3/objects/contacts&retries=2")
//!     .build()?;
//! camel.add_route_definition(route).await?;
//! camel.start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! The `harvest_camel` example (`examples/harvest-camel` in the repository)
//! calls this component from harvest activities.
//!
//! # Endpoint URI
//!
//! `san-mateo:<integration>?<options>`. The path is the integration key
//! (Nango: `provider_config_key`). [`uri_options`] lists the options:
//!
//! | Option | Default | Meaning |
//! |---|---|---|
//! | `connectionId` | none | The connection id. With `allowHeaderOverrides`, the [`HEADER_CONNECTION_ID`] header replaces it. |
//! | `environment` | component default | The San Mateo environment. |
//! | `method` | `GET` | The HTTP method. With `allowHeaderOverrides`, the [`HEADER_METHOD`] header replaces it. |
//! | `path` | none | The upstream path and query. With `allowHeaderOverrides`, the [`HEADER_PATH`] header replaces it. |
//! | `retries` | `0` | Retries after the first attempt, 0 to 9. |
//! | `retryOn` | none | Status codes to retry also for non-idempotent methods, for example `500,502`. |
//! | `baseUrlOverride` | none | A base URL in place of the provider base URL. San Mateo checks the host. |
//! | `throwExceptionOnFailure` | `true` | Fail the exchange on a non-2xx reply. |
//! | `maxBodySize` | 2 MiB | The largest request body in bytes. |
//! | `allowHeaderOverrides` | `false` | Let the headers below replace `connectionId`, `method` and `path`. |
//!
//! Percent-encode `?` and `&` in a `path` value, for example
//! `path=/v3/contacts%3Flimit%3D10`.
//!
//! # Headers
//!
//! | Header | Direction | Meaning |
//! |---|---|---|
//! | [`HEADER_CONNECTION_ID`] | in | Replaces `connectionId` (needs `allowHeaderOverrides`). |
//! | [`HEADER_METHOD`] | in | Replaces `method` (needs `allowHeaderOverrides`). |
//! | [`HEADER_PATH`] | in | Replaces `path` (needs `allowHeaderOverrides`). The value can have a query. |
//! | `SanMateoProxy-<Name>` | in | Goes upstream as `<Name>`. |
//! | [`HEADER_RESPONSE_CODE`] | out | The reply status as a number. |
//! | `SanMateoResponse-<name>` | out | One reply header. Secret headers stay out. |
//!
//! # Bodies
//!
//! The in body goes upstream. A JSON body becomes JSON bytes with
//! `content-type: application/json`, unless a `SanMateoProxy-Content-Type`
//! header sets a different type. An XML body gets `application/xml` in the
//! same way. A text body becomes UTF-8 bytes. A byte or stream body goes as
//! it is. An empty body sends no body. A body larger than `maxBodySize`
//! fails the exchange with [`CamelError::StreamLimitExceeded`].
//!
//! The reply replaces the in body. A JSON reply becomes a JSON body. A text or
//! XML reply becomes a text body. Other replies become a byte body. The
//! producer clears the out message, so the in message is the only result.
//!
//! # Errors
//!
//! With `throwExceptionOnFailure=true`, a non-2xx reply fails the exchange
//! with [`CamelError::HttpOperationFailed`]. The error has the status and the
//! reason phrase. It has no reply body, because a body can hold secrets.
//! [`to_camel_error`] maps a [`san_mateo::Error`] to a camel error. The
//! harvest-camel `is_retryable` function gives the same answer as
//! [`san_mateo::Error::is_retryable`] for each mapped error. The
//! `harvest_camel` example (`examples/harvest-camel`) calls this component
//! from harvest activities, and its test checks this rule with the real
//! harvest-camel.
//!
//! [`SanMateo`]: san_mateo::SanMateo
//! [`CamelError::StreamLimitExceeded`]: camel_api::CamelError::StreamLimitExceeded
//! [`CamelError::HttpOperationFailed`]: camel_api::CamelError::HttpOperationFailed

// The camel traits take and return `CamelError` by value. It is large, but a
// box does not match the trait signatures.
#![allow(clippy::result_large_err)]

mod component;
mod error;
mod options;
mod producer;

pub use component::SanMateoComponent;
pub use error::to_camel_error;
pub use options::{EndpointOptions, METHODS, uri_options};

/// The camel URI scheme of this component.
pub const SCHEME: &str = "san-mateo";

/// The default largest request body: 2 MiB, the harvest input limit.
pub const DEFAULT_MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// In header: the connection id. It replaces the `connectionId` option only
/// with `allowHeaderOverrides=true`.
pub const HEADER_CONNECTION_ID: &str = "SanMateoConnectionId";

/// In header: the upstream path and query. It replaces the `path` option
/// only with `allowHeaderOverrides=true`.
pub const HEADER_PATH: &str = "SanMateoPath";

/// In header: the HTTP method. It replaces the `method` option only with
/// `allowHeaderOverrides=true`.
pub const HEADER_METHOD: &str = "SanMateoMethod";

/// In header prefix: `SanMateoProxy-<Name>` goes upstream as `<Name>`.
pub const HEADER_PROXY_PREFIX: &str = "SanMateoProxy-";

/// Out header prefix: each reply header comes back as `SanMateoResponse-<name>`.
pub const HEADER_RESPONSE_PREFIX: &str = "SanMateoResponse-";

/// Out header: the reply status as a number. The camel HTTP component uses
/// the same name.
pub const HEADER_RESPONSE_CODE: &str = "CamelHttpResponseCode";
