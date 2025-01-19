use crate::{
    config::JellyfinConfig,
    http::{Item, ItemsFilter, JellyfinClient},
};
use std::sync::Arc;

/// This is a high level service that interacts with Jellyfin API and transforms
/// the data into a more usable format.
pub struct Jellyfin {
    client: JellyfinClient,
    username: String,
}

impl Jellyfin {
    pub fn new(username: &str, config: &JellyfinConfig) -> anyhow::Result<Arc<Self>> {
        let client = JellyfinClient::new(&config.base_url, &config.api_key)?;
        let it = Self {
            client,
            username: username.to_string(),
        };
        let it = Arc::new(it);
        Ok(it)
    }

    pub async fn user_id(&self) -> anyhow::Result<String> {
        let users = self.client.users().await?;
        users
            .into_iter()
            .find(|user| user.name == self.username)
            .map(|u| u.id)
            .ok_or_else(|| anyhow::anyhow!("User {} not found", self.username))
    }

    pub async fn items(&self, filter: ItemsFilter<'_>) -> anyhow::Result<Vec<Item>> {
        self.client.items(filter).await
    }

    pub async fn watched_items(&self, item_types: &[&str]) -> anyhow::Result<Vec<Item>> {
        let user_id = self.user_id().await?;
        let filter = ItemsFilter::new()
            .user_id(&user_id)
            .recursive()
            .played()
            .favorite(false)
            .fields(&["ProviderIds"])
            .include_item_types(item_types);
        self.items(filter).await
    }
}
