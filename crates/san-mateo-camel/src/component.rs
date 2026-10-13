//! The `san-mateo:` camel component and its endpoint.

use std::sync::Arc;

use async_trait::async_trait;
use camel_api::{BoxProcessor, CamelError};
use camel_component_api::{
    Component, ComponentContext, ComponentMetadata, Consumer, Endpoint, ProducerContext,
    RuntimeObservability,
};
use san_mateo::SanMateo;

use crate::options::EndpointOptions;
use crate::producer::Producer;

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
    pub fn new(san: SanMateo) -> Self {
        let _ = san;
        todo!()
    }

    /// Sets the environment for URIs with no `environment` option.
    #[must_use]
    pub fn environment(self, environment: impl Into<String>) -> Self {
        let _ = (self, environment.into());
        todo!()
    }

    /// Sets the largest request body for URIs with no `maxBodySize` option.
    /// The default is [`crate::DEFAULT_MAX_BODY_SIZE`].
    #[must_use]
    pub fn max_body_size(self, bytes: usize) -> Self {
        let _ = (self, bytes);
        todo!()
    }

    /// Parses and checks the options of a `san-mateo:` URI.
    ///
    /// # Errors
    ///
    /// [`CamelError::InvalidUri`] for a URI problem. [`CamelError::Config`]
    /// when no environment is set or the environment does not exist.
    pub fn endpoint_options(&self, uri: &str) -> Result<EndpointOptions, CamelError> {
        let _ = (&self.san, &self.environment, self.max_body_size, uri);
        todo!()
    }
}

#[async_trait]
impl Component for SanMateoComponent {
    fn scheme(&self) -> &str {
        todo!()
    }

    fn metadata(&self) -> ComponentMetadata {
        todo!()
    }

    fn create_endpoint(
        &self,
        uri: &str,
        _ctx: &dyn ComponentContext,
    ) -> Result<Box<dyn Endpoint>, CamelError> {
        let _ = uri;
        todo!()
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
        todo!()
    }

    fn create_producer(
        &self,
        _rt: Arc<dyn RuntimeObservability>,
        _ctx: &ProducerContext,
    ) -> Result<BoxProcessor, CamelError> {
        let _ = &self.producer;
        todo!()
    }
}
