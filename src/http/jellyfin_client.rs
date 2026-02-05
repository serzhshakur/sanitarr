use super::ResponseExt;
use crate::config::JellyfinConfig;
use anyhow::Ok;
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Client, ClientBuilder, Url};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct JellyfinClient {
    client: Client,
    base_url: Url,
}

impl JellyfinClient {
    pub fn new(config: &JellyfinConfig) -> anyhow::Result<Self> {
        let JellyfinConfig { base_url, api_key } = config;
        let base_url = Url::parse(base_url)?;
        let default_headers = auth_headers(api_key)?;
        let client = ClientBuilder::new()
            .default_headers(default_headers)
            .build()?;
        Ok(Self { client, base_url })
    }

    /// Get all items that match the given query filter
    /// https://api.jellyfin.org/#tag/Items
    pub async fn items(&self, items_filter: ItemsFilter<'_>) -> anyhow::Result<Vec<Item>> {
        let url = self.base_url.join("Items")?;

        // pagination
        let mut items = Vec::new();
        let mut start_index: usize = 0;
        let limit = 100;

        loop {
            let response = self
                .client
                .get(url.clone())
                .query(&items_filter)
                .query(&[("startIndex", start_index), ("limit", limit)])
                .send()
                .await?
                .handle_error()
                .await?
                .json::<ItemsResponse>()
                .await?;

            if response.items.is_empty() {
                break;
            }

            items.extend(response.items);

            if items.len() >= response.total_record_count {
                break;
            }
            start_index = items.len();
        }

        Ok(items)
    }

    /// Get all users.
    /// https://api.jellyfin.org/#tag/User
    async fn users(&self) -> anyhow::Result<Vec<User>> {
        let url = self.base_url.join("Users")?;
        let response = self
            .client
            .get(url)
            .send()
            .await?
            .handle_error()
            .await?
            .json::<Vec<User>>()
            .await?;

        Ok(response)
    }

    /// Get a user by it's username (not id). Throws an error if the user not
    /// found
    pub async fn user(&self, user_name: &str) -> anyhow::Result<User> {
        self.users()
            .await?
            .into_iter()
            .find(|user| user.name == user_name)
            .ok_or_else(|| anyhow::anyhow!("User {user_name} not found"))
    }
}

fn auth_headers(api_key: &str) -> Result<HeaderMap, anyhow::Error> {
    let mut auth_headers = HeaderMap::new();
    let header_value = format!("MediaBrowser Token={api_key}");
    let mut header_value = HeaderValue::from_str(&header_value)?;
    header_value.set_sensitive(true);
    auth_headers.insert(AUTHORIZATION, header_value);
    Ok(auth_headers)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsResponse {
    pub items: Vec<Item>,
    total_record_count: usize,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub name: String,
    pub id: String,
    pub series_id: Option<String>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    provider_ids: Option<ProviderIds>,
    user_data: Option<ItemUserData>,
}

impl Item {
    pub fn tmdb_id(&self) -> Option<&str> {
        self.provider_ids.as_ref()?.tmdb.as_deref()
    }

    pub fn tvdb_id(&self) -> Option<&str> {
        self.provider_ids.as_ref()?.tvdb.as_deref()
    }

    pub fn last_played_date(&self) -> Option<DateTime<Utc>> {
        self.user_data.as_ref()?.last_played_date
    }

    pub fn watched(&self) -> bool {
        self.user_data
            .as_ref()
            .map(|ud| ud.played)
            .unwrap_or_default()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(test, derive(Default))]
pub struct ProviderIds {
    tmdb: Option<String>,
    tvdb: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[cfg_attr(test, derive(Default))]
pub struct ItemUserData {
    last_played_date: Option<DateTime<Utc>>,
    played: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct UserId(String);

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    pub id: UserId,
    name: String,
}

/// Filter for querying items. Serializes into query parameters. Check [docs]
/// for more details
///
/// [docs]: https://api.jellyfin.org/#tag/Items/operation/GetItems
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemsFilter<'a> {
    #[serde(serialize_with = "to_comma_separated")]
    fields: Option<&'a [&'a str]>,
    #[serde(serialize_with = "to_comma_separated")]
    include_item_types: Option<&'a [&'a str]>,
    #[serde(
        serialize_with = "to_comma_separated",
        skip_serializing_if = "Option::is_none"
    )]
    ids: Option<&'a [&'a str]>,
    is_favorite: Option<bool>,
    is_played: Option<bool>,
    recursive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a str>,
}

impl<'a> ItemsFilter<'a> {
    pub fn new() -> Self {
        Self {
            fields: None,
            include_item_types: None,
            is_favorite: None,
            is_played: None,
            recursive: None,
            user_id: None,
            ids: None,
        }
    }

    #[must_use]
    pub fn user_id(mut self, user_id: &'a str) -> Self {
        self.user_id = Some(user_id);
        self
    }

    #[must_use]
    pub fn played(mut self) -> Self {
        self.is_played = Some(true);
        self
    }

    #[must_use]
    pub fn recursive(mut self) -> Self {
        self.recursive = Some(true);
        self
    }

    #[must_use]
    pub fn favorite(mut self, value: bool) -> Self {
        self.is_favorite = Some(value);
        self
    }

    #[must_use]
    pub fn include_item_types(mut self, types: &'a [&'a str]) -> Self {
        self.include_item_types = Some(types);
        self
    }

    #[must_use]
    pub fn fields(mut self, fields: &'a [&'a str]) -> Self {
        self.fields = Some(fields);
        self
    }
    #[must_use]
    pub fn ids(mut self, ids: &'a [&'a str]) -> Self {
        self.ids = Some(ids);
        self
    }

    /// a convenience function to filter out watched items
    pub fn watched() -> Self {
        Self::new()
            .recursive()
            .played()
            .favorite(false)
            .fields(&["ProviderIds"])
    }
}

fn to_comma_separated<'a, S>(
    values: &Option<&'a [&'a str]>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(values) = values
        && !values.is_empty()
    {
        let values = values.join(",");
        return serializer.serialize_some(&values);
    }
    serializer.serialize_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_items_filter() {
        let filter = ItemsFilter::new()
            .user_id("user_id")
            .recursive()
            .played()
            .favorite(false)
            .include_item_types(&["Movie", "Video"])
            .fields(&["ProviderIds", "Path"]);

        let expected = r#"{"fields":"ProviderIds,Path","includeItemTypes":"Movie,Video","isFavorite":false,"isPlayed":true,"recursive":true,"userId":"user_id"}"#;
        let actual = serde_json::to_string(&filter).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_auth_headers() -> anyhow::Result<()> {
        let headers = auth_headers("abc")?;
        let expected = "MediaBrowser Token=abc";
        let actual = headers.get(AUTHORIZATION).unwrap().to_str()?;
        assert_eq!(expected, actual);
        Ok(())
    }
}
