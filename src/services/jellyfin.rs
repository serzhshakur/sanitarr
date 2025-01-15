use crate::{
    config::JellyfinConfig,
    http::{Item, ItemsFilter, JellyfinClient},
};
use std::sync::Arc;

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
        let users = self.client.get_users().await?;
        users
            .into_iter()
            .find(|user| user.name == self.username)
            .map(|u| u.id)
            .ok_or_else(|| anyhow::anyhow!("User {} not found", self.username))
    }

    pub async fn query_items(&self, filter: ItemsFilter<'_>) -> anyhow::Result<Vec<Item>> {
        self.client.get_items(filter).await
    }

    pub async fn query_watched(&self, item_types: &[&str]) -> anyhow::Result<Vec<Item>> {
        let user_id = self.user_id().await?;
        let filter = ItemsFilter::new()
            .user_id(&user_id)
            .recursive()
            .played()
            .favorite(false)
            .fields(&["ProviderIds", "Path"])
            .include_item_types(item_types);
        self.client.get_items(filter).await
    }
}
