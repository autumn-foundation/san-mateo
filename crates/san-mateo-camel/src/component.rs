//! The `san-mateo:` camel component and its endpoint.

use std::sync::Arc;

use async_trait::async_trait;
use camel_api::{BoxProcessor, BoxProcessorExt, CamelError};
use camel_component_api::{
    Component, ComponentCapabilities, ComponentContext, ComponentMetadata, Consumer, Endpoint,
    ProducerContext, RuntimeObservability,
};
use san_mateo::SanMateo;

use crate::options::{EndpointOptions, uri_options};
use crate::producer::Producer;
use crate::{DEFAULT_MAX_BODY_SIZE, SCHEME};

/// The `san-mateo:` camel component. It is producer-only.
///
/// ```no_run
/// # fn demo(san: san_mateo::SanMateo) {
/// let component = san_mateo_camel::SanMateoComponent::new(san).environment("prod");
/// # let _ = component;
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SanMateoComponent {
    san: SanMateo,
    environment: Option<String>,
    max_body_size: usize,
}

impl SanMateoComponent {
    /// Makes a component for a San Mateo service. It has no default
    /// environment, so each URI must set `environment`.
    #[must_use]
    pub const fn new(san: SanMateo) -> Self {
        Self {
            san,
            environment: None,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
        }
    }

    /// Sets the environment for URIs with no `environment` option.
    #[must_use]
    pub fn environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Sets the largest request body for URIs with no `maxBodySize` option.
    /// The default is [`crate::DEFAULT_MAX_BODY_SIZE`]. A value of 0 allows
    /// only an empty body.
    #[must_use]
    pub const fn max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }

    /// Parses and checks the options of a `san-mateo:` URI.
    ///
    /// # Errors
    ///
    /// [`CamelError::InvalidUri`] for a URI problem. [`CamelError::Config`]
    /// when no environment is set or the environment does not exist.
    pub fn endpoint_options(&self, uri: &str) -> Result<EndpointOptions, CamelError> {
        let options = EndpointOptions::parse(uri, self.environment.as_deref(), self.max_body_size)?;
        // Fail at route start, not at the first message.
        self.san.environment(&options.environment).map_err(|_| {
            CamelError::Config(format!(
                "the San Mateo environment '{}' does not exist",
                options.environment
            ))
        })?;
        Ok(options)
    }
}

#[async_trait]
impl Component for SanMateoComponent {
    fn scheme(&self) -> &str {
        SCHEME
    }

    fn metadata(&self) -> ComponentMetadata {
        let mut metadata = ComponentMetadata::minimal(SCHEME)
            .with_description(
                "Calls provider APIs through the San Mateo proxy with stored, refreshed auth.",
            )
            .with_capabilities(ComponentCapabilities {
                supports_producer: true,
                ..ComponentCapabilities::default()
            })
            .with_uri_options(uri_options());
        env!("CARGO_PKG_VERSION").clone_into(&mut metadata.version);
        "san-mateo:integration".clone_into(&mut metadata.uri_syntax);
        metadata
    }

    fn create_endpoint(
        &self,
        uri: &str,
        _ctx: &dyn ComponentContext,
    ) -> Result<Box<dyn Endpoint>, CamelError> {
        let options = self.endpoint_options(uri)?;
        Ok(Box::new(SanMateoEndpoint {
            uri: uri.to_owned(),
            producer: Producer::new(self.san.clone(), Arc::new(options)),
        }))
    }
}

/// One `san-mateo:` endpoint.
struct SanMateoEndpoint {
    uri: String,
    producer: Producer,
}

impl Endpoint for SanMateoEndpoint {
    fn uri(&self) -> &str {
        &self.uri
    }

    fn create_consumer(
        &self,
        _rt: Arc<dyn RuntimeObservability>,
    ) -> Result<Box<dyn Consumer>, CamelError> {
        Err(CamelError::EndpointCreationFailed(format!(
            "{SCHEME}: is producer-only; use it in to(...), not in from(...)"
        )))
    }

    fn create_producer(
        &self,
        _rt: Arc<dyn RuntimeObservability>,
        _ctx: &ProducerContext,
    ) -> Result<BoxProcessor, CamelError> {
        let producer = self.producer.clone();
        Ok(BoxProcessor::from_fn(move |exchange| {
            let producer = producer.clone();
            async move { producer.send(exchange).await }
        }))
    }
}
