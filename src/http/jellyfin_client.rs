use super::ResponseExt;
use anyhow::Ok;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

pub struct JellyfinClient {
    client: Client,
    base_url: Url,
    default_headers: HeaderMap,
}

impl JellyfinClient {
    pub fn new(base_url: &str, api_key: &str) -> anyhow::Result<Self> {
        let base_url = Url::parse(base_url)?;

        let mut default_headers = HeaderMap::new();
        let header_value = format!("MediaBrowser Token={api_key}");
        let mut header_value = HeaderValue::from_str(&header_value)?;
        header_value.set_sensitive(true);
        default_headers.insert(AUTHORIZATION, header_value);

        Ok(Self {
            client: Client::new(),
            base_url,
            default_headers,
        })
    }

    pub async fn get_items(&self, items_filter: ItemsFilter<'_>) -> anyhow::Result<Vec<Item>> {
        let url = self.base_url.join("Items")?;
        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .query(&items_filter)
            .send()
            .await?
            .handle_error()
            .await?
            .json::<ItemsResponse>()
            .await?;

        Ok(response.items)
    }

    pub async fn get_users(&self) -> anyhow::Result<Vec<User>> {
        let url = self.base_url.join("Users")?;
        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json::<Vec<User>>()
            .await?;

        Ok(response)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsResponse {
    pub items: Vec<Item>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub name: String,
    pub path: String,
    #[serde(rename = "Type")]
    pub item_type: ItemType,
    pub provider_ids: ProviderIds,
}

impl Item {
    pub fn tmdb_id(&self) -> Option<&str> {
        self.provider_ids.tmdb.as_deref()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ProviderIds {
    pub tmdb: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub enum ItemType {
    Movie,
    Episode,
    Season,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemsFilter<'a> {
    #[serde(serialize_with = "to_comma_separated")]
    fields: Option<&'a [&'a str]>,
    #[serde(serialize_with = "to_comma_separated")]
    include_item_types: Option<&'a [&'a str]>,
    is_favorite: Option<bool>,
    is_played: Option<bool>,
    recursive: Option<bool>,
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
        }
    }

    pub fn user_id(mut self, user_id: &'a str) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn played(mut self) -> Self {
        self.is_played = Some(true);
        self
    }

    pub fn recursive(mut self) -> Self {
        self.recursive = Some(true);
        self
    }

    pub fn favorite(mut self, value: bool) -> Self {
        self.is_favorite = Some(value);
        self
    }

    pub fn include_item_types(mut self, types: &'a [&'a str]) -> Self {
        self.include_item_types = Some(types);
        self
    }

    pub fn fields(mut self, fields: &'a [&'a str]) -> Self {
        self.fields = Some(fields);
        self
    }
}

fn to_comma_separated<'a, S>(
    values: &Option<&'a [&'a str]>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(values) = values {
        if !values.is_empty() {
            let values = values.join(",");
            return serializer.serialize_some(&values);
        }
    }
    serializer.serialize_none()
}
