//! Integrations: one provider with its OAuth client for one environment.

use super::{Integration, IntegrationPatch, NewIntegration, SanMateo, invalid};
use crate::error::{Error, Result};
use crate::secret::Secret;
use crate::store::IntegrationRow;

/// Returns `true` for a valid integration key: 1 to 100 chars of
/// `[A-Za-z0-9_.-]`.
pub(crate) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 100
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

impl SanMateo {
    fn to_integration(&self, row: IntegrationRow) -> Integration {
        let display_name = row.display_name.clone().unwrap_or_else(|| {
            self.catalog()
                .get(&row.provider)
                .map_or_else(|| row.provider.clone(), |p| p.display_name.clone())
        });
        Integration {
            unique_key: row.unique_key,
            provider: row.provider,
            display_name,
            client_id: row.client_id,
            has_client_secret: row.client_secret.is_some(),
            scopes: row.scopes,
            custom: row.custom,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn seal_client_secret(&self, env: &str, key: &str, secret: Option<&Secret>) -> Result<Option<String>> {
        secret
            .filter(|s| !s.is_empty())
            .map(|s| self.inner.keys.seal(s.expose().as_bytes(), &Self::integration_aad(env, key)))
            .transpose()
    }

    /// Returns the stored integration row.
    pub(crate) async fn integration_row(&self, env: &str, key: &str) -> Result<IntegrationRow> {
        self.inner
            .store
            .get_integration(env, key)
            .await?
            .ok_or_else(|| Error::UnknownIntegration(key.to_owned()))
    }

    /// Returns the plaintext client secret of an integration row.
    pub(crate) fn client_secret(&self, row: &IntegrationRow) -> Result<Option<Secret>> {
        row.client_secret
            .as_deref()
            .map(|sealed| {
                let bytes = self
                    .inner
                    .keys
                    .open(sealed, &Self::integration_aad(&row.environment, &row.unique_key))?;
                String::from_utf8(bytes).map(Secret::new).map_err(|_| Error::Crypto)
            })
            .transpose()
    }

    /// Adds an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownProvider`], [`Error::Conflict`], [`Error::Invalid`].
    pub async fn create_integration(&self, env: &str, input: NewIntegration) -> Result<Integration> {
        self.environment(env)?;
        if !valid_key(&input.unique_key) {
            return Err(invalid("the integration key must be 1 to 100 chars of [A-Za-z0-9_.-]"));
        }
        self.catalog().require(&input.provider)?;
        let now = self.now();
        let row = IntegrationRow {
            environment: env.to_owned(),
            client_secret: self.seal_client_secret(env, &input.unique_key, input.client_secret.as_ref())?,
            unique_key: input.unique_key,
            provider: input.provider,
            display_name: input.display_name.filter(|d| !d.is_empty()),
            client_id: input.client_id.filter(|c| !c.is_empty()),
            scopes: input.scopes,
            custom: input.custom,
            created_at: now,
            updated_at: now,
        };
        self.inner.store.insert_integration(row.clone()).await?;
        Ok(self.to_integration(row))
    }

    /// Reads an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn get_integration(&self, env: &str, key: &str) -> Result<Integration> {
        Ok(self.to_integration(self.integration_row(env, key).await?))
    }

    /// Lists the integrations of an environment.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn list_integrations(&self, env: &str) -> Result<Vec<Integration>> {
        let rows = self.inner.store.list_integrations(env).await?;
        Ok(rows.into_iter().map(|r| self.to_integration(r)).collect())
    }

    /// Changes an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn update_integration(
        &self,
        env: &str,
        key: &str,
        patch: IntegrationPatch,
    ) -> Result<Integration> {
        let mut row = self.integration_row(env, key).await?;
        if let Some(d) = patch.display_name {
            row.display_name = Some(d).filter(|d| !d.is_empty());
        }
        if let Some(c) = patch.client_id {
            row.client_id = Some(c).filter(|c| !c.is_empty());
        }
        if let Some(secret) = patch.client_secret {
            row.client_secret = self.seal_client_secret(env, key, Some(&secret))?;
        }
        if let Some(scopes) = patch.scopes {
            row.scopes = scopes;
        }
        if let Some(custom) = patch.custom {
            row.custom = custom;
        }
        row.updated_at = self.now();
        if !self.inner.store.update_integration(row.clone()).await? {
            return Err(Error::UnknownIntegration(key.to_owned()));
        }
        Ok(self.to_integration(row))
    }

    /// Deletes an integration and its connections.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn delete_integration(&self, env: &str, key: &str) -> Result<()> {
        if self.inner.store.delete_integration(env, key).await? {
            Ok(())
        } else {
            Err(Error::UnknownIntegration(key.to_owned()))
        }
    }
}
