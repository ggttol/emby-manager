use anyhow::{Context, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct EmbyClient {
    base_url: String,
    api_key: String,
    http: Client,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbyVirtualFolder {
    #[serde(rename = "ItemId")]
    pub item_id: Option<String>,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "CollectionType")]
    pub collection_type: Option<String>,
    #[serde(rename = "Locations", default)]
    pub locations: Vec<String>,
    #[serde(rename = "LibraryOptions")]
    pub library_options: Option<EmbyLibraryOptions>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbyLibraryOptions {
    #[serde(rename = "PathInfos", default)]
    pub path_infos: Vec<EmbyPathInfo>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbyPathInfo {
    #[serde(rename = "Path")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbyItem {
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub item_type: Option<String>,
    #[serde(rename = "Path")]
    pub path: Option<String>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: BTreeMap<String, Value>,
    #[serde(rename = "ProviderIds", default)]
    pub provider_ids: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct EmbyItemsResult {
    pub items: Vec<EmbyItem>,
    pub total_record_count: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
struct EmbyItemsPage {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyItem>,
    #[serde(rename = "TotalRecordCount")]
    total_record_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
pub struct EmbyLibrary {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub library_type: String,
    pub paths: Vec<String>,
}

impl EmbyClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, http: Client) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into().trim().to_string(),
            http,
        }
    }

    pub fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub async fn virtual_folders(&self) -> anyhow::Result<Vec<EmbyVirtualFolder>> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}/Library/VirtualFolders", self.base_url);
        Ok(self
            .http
            .get(url)
            .query(&[("api_key", self.api_key.as_str())])
            .send()
            .await
            .context("failed to call Emby /Library/VirtualFolders")?
            .error_for_status()
            .context("Emby /Library/VirtualFolders returned an error")?
            .json()
            .await?)
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<EmbyLibrary>> {
        Ok(self
            .virtual_folders()
            .await?
            .into_iter()
            .map(EmbyLibrary::from)
            .collect())
    }

    pub async fn poster_items(
        &self,
        parent_id: &str,
        item_types: &str,
        limit: usize,
    ) -> anyhow::Result<EmbyItemsResult> {
        if parent_id.trim().is_empty() {
            bail!("parent_id is required for Emby item listing");
        }
        if item_types.trim().is_empty() {
            bail!("item_types is required for Emby item listing");
        }
        let limit = limit.clamp(1, 100_000);
        let mut start = 0usize;
        let mut items = Vec::new();
        let mut total_record_count = None;

        while items.len() < limit {
            let page_limit = (limit - items.len()).min(1000);
            let page = self
                .items_page(
                    parent_id,
                    item_types,
                    "ProviderIds,Path,ImageTags",
                    start,
                    page_limit,
                )
                .await?;
            if total_record_count.is_none() {
                total_record_count = page.total_record_count;
            }
            if page.items.is_empty() {
                break;
            }
            start += page.items.len();
            items.extend(page.items);
            if let Some(total) = total_record_count
                && start >= total
            {
                break;
            }
        }

        let truncated = total_record_count.is_some_and(|total| items.len() < total);
        Ok(EmbyItemsResult {
            items,
            total_record_count,
            truncated,
        })
    }

    pub async fn refresh_library(&self) -> anyhow::Result<u16> {
        self.post_empty("/Library/Refresh", &[]).await
    }

    pub async fn refresh_item(
        &self,
        item_id: &str,
        recursive: bool,
        full: bool,
    ) -> anyhow::Result<u16> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby refresh");
        }
        let path = format!("/Items/{}/Refresh", urlencoding::encode(item_id.trim()));
        let refresh_mode = if full { "FullRefresh" } else { "Default" };
        self.post_empty(
            &path,
            &[
                ("Recursive", if recursive { "true" } else { "false" }),
                ("MetadataRefreshMode", refresh_mode),
                ("ImageRefreshMode", refresh_mode),
                ("ReplaceAllMetadata", if full { "true" } else { "false" }),
                ("ReplaceAllImages", if full { "true" } else { "false" }),
            ],
        )
        .await
    }

    async fn post_empty(&self, path: &str, params: &[(&str, &str)]) -> anyhow::Result<u16> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}{}", self.base_url, path);
        let mut query = Vec::with_capacity(params.len() + 1);
        query.push(("api_key", self.api_key.as_str()));
        query.extend_from_slice(params);
        let status = self
            .http
            .post(url)
            .query(&query)
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .status();
        Ok(status.as_u16())
    }

    async fn items_page(
        &self,
        parent_id: &str,
        item_types: &str,
        fields: &str,
        start_index: usize,
        limit: usize,
    ) -> anyhow::Result<EmbyItemsPage> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}/Items", self.base_url);
        Ok(self
            .http
            .get(url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("ParentId", parent_id.trim()),
                ("Recursive", "true"),
                ("IncludeItemTypes", item_types.trim()),
                ("Fields", fields),
                ("SortBy", "SortName"),
                ("StartIndex", &start_index.to_string()),
                ("Limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("failed to call Emby /Items")?
            .error_for_status()
            .context("Emby /Items returned an error")?
            .json()
            .await?)
    }
}

impl EmbyItem {
    pub fn provider_id(&self, key: &str) -> Option<String> {
        self.provider_ids
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .and_then(|(_, value)| match value {
                Value::String(s) => {
                    let s = s.trim();
                    (!s.is_empty()).then(|| s.to_string())
                }
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
    }

    pub fn has_primary_image(&self) -> bool {
        self.image_tags
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Primary"))
            .is_some_and(|(_, value)| match value {
                Value::String(s) => !s.trim().is_empty(),
                Value::Null => false,
                _ => true,
            })
    }
}

impl From<EmbyVirtualFolder> for EmbyLibrary {
    fn from(folder: EmbyVirtualFolder) -> Self {
        let mut paths = Vec::new();
        for path in folder.locations {
            push_path(&mut paths, path);
        }
        if let Some(options) = folder.library_options {
            for info in options.path_infos {
                if let Some(path) = info.path {
                    push_path(&mut paths, path);
                }
            }
        }
        Self {
            id: folder
                .item_id
                .and_then(|id| (!id.trim().is_empty()).then(|| id.trim().to_string())),
            name: if folder.name.trim().is_empty() {
                "(unnamed)".to_string()
            } else {
                folder.name
            },
            library_type: folder
                .collection_type
                .unwrap_or_else(|| "mixed".to_string()),
            paths,
        }
    }
}

fn push_path(paths: &mut Vec<String>, path: String) {
    let path = path.trim();
    if !path.is_empty() && !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}
