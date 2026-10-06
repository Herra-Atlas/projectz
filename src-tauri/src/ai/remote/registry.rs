use std::sync::Arc;

use tracing::info;

use super::types::Endpoint;
use crate::database::Database;

pub struct EndpointRegistry {
    database: Arc<Database>,
}

impl EndpointRegistry {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }

    pub fn list(&self) -> Vec<Endpoint> {
        self.database.list_endpoints().unwrap_or_default()
    }

    pub fn add(&self, endpoint: Endpoint) -> Result<(), String> {
        self.database.save_endpoint(&endpoint)?;
        info!("endpoint added");
        Ok(())
    }

    pub fn update(&self, endpoint: Endpoint) -> Result<bool, String> {
        if !self
            .list()
            .iter()
            .any(|existing| existing.id == endpoint.id)
        {
            return Ok(false);
        }
        self.database.save_endpoint(&endpoint)?;
        info!("endpoint updated");
        Ok(true)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.database.remove_endpoint(id)?;
        info!(id, "endpoint removed");
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Endpoint> {
        self.database
            .list_endpoints()
            .ok()?
            .into_iter()
            .find(|endpoint| endpoint.id == id)
    }
}
