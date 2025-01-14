use crate::{
    config::JellyfinConfig,
    http::{
        jellyfin_client::{Item, ItemsFilter},
        JellyfinClient,
    },
};

pub struct Jellyfin {
    client: JellyfinClient,
    username: String,
}

impl Jellyfin {
    pub fn new(username: &str, config: &JellyfinConfig) -> anyhow::Result<Self> {
        let client = JellyfinClient::new(&config.base_url, &config.api_key)?;
        Ok(Self {
            client,
            username: username.to_string(),
        })
    }

    pub async fn get_user_id(&self, username: &str) -> anyhow::Result<String> {
        let users = self.client.get_users().await?;
        users
            .into_iter()
            .find(|user| user.name == username)
            .map(|u| u.id)
            .ok_or_else(|| anyhow::anyhow!("User {username} not found"))
    }

    pub async fn get_watched_items(&self) -> anyhow::Result<Vec<Item>> {
        let user_id = self.get_user_id(&self.username).await?;
        let items_filter = ItemsFilter::new()
            .user_id(&user_id)
            .recursive()
            .played()
            .include_item_types(&["Movie", "Video"])
            .fields(&["ProviderIds", "Path"]);

        self.client.get_items(items_filter).await
    }
}
