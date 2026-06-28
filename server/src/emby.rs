use anyhow::{Context, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct EmbyClient {
    base_url: String,
    api_key: String,
    http: Client,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbyLibraryOptions {
    #[serde(rename = "PathInfos", default)]
    pub path_infos: Vec<EmbyPathInfo>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    #[serde(rename = "ProductionYear")]
    pub production_year: Option<i32>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: BTreeMap<String, Value>,
    #[serde(rename = "ProviderIds", default)]
    pub provider_ids: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbyEpisode {
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "ParentIndexNumber")]
    pub parent_index_number: Option<i32>,
    #[serde(rename = "IndexNumber")]
    pub index_number: Option<i32>,
    #[serde(rename = "LocationType")]
    pub location_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmbyItemsResult {
    pub items: Vec<EmbyItem>,
    pub total_record_count: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct EmbyCleanupItemsResult {
    pub items: Vec<EmbyCleanupItem>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
struct EmbyItemsPage {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyItem>,
    #[serde(rename = "TotalRecordCount")]
    total_record_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EmbyEpisodesPage {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyEpisode>,
}

#[derive(Debug, Deserialize)]
struct EmbyCleanupItemsPage {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyCleanupItem>,
    #[serde(rename = "TotalRecordCount")]
    total_record_count: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbyCleanupItem {
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Path")]
    pub path: Option<String>,
    #[serde(rename = "ProductionYear")]
    pub production_year: Option<i32>,
    #[serde(rename = "CommunityRating")]
    pub community_rating: Option<f64>,
    #[serde(rename = "CriticRating")]
    pub critic_rating: Option<f64>,
    #[serde(rename = "UserData")]
    pub user_data: Option<EmbyCleanupUserData>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: BTreeMap<String, Value>,
    #[serde(rename = "ProviderIds", default)]
    pub provider_ids: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbyCleanupUserData {
    #[serde(rename = "Rating")]
    pub rating: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct EmbyRemoteSearchCandidate {
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "ProductionYear")]
    pub production_year: Option<i32>,
    #[serde(rename = "ProviderIds", default)]
    pub provider_ids: BTreeMap<String, Value>,
    #[serde(rename = "ImageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "Overview")]
    pub overview: Option<String>,
}

#[derive(Debug, Serialize)]
struct RemoteSearchInfo<'a> {
    #[serde(rename = "Name")]
    name: &'a str,
    #[serde(rename = "ProviderIds")]
    provider_ids: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct RemoteSearchRequest<'a> {
    #[serde(rename = "SearchInfo")]
    search_info: RemoteSearchInfo<'a>,
    #[serde(rename = "ItemId")]
    item_id: &'a str,
    #[serde(rename = "IncludeDisabledProviders")]
    include_disabled_providers: bool,
}

#[derive(Debug, Serialize)]
struct RemoteSearchApplyRequest<'a> {
    #[serde(rename = "ProviderIds")]
    provider_ids: BTreeMap<&'static str, &'a str>,
}

#[derive(Debug, Serialize)]
struct EmbyCreateUserRequest<'a> {
    #[serde(rename = "Name")]
    name: &'a str,
}

#[derive(Debug, Serialize)]
struct EmbySetPasswordRequest<'a> {
    #[serde(rename = "Id")]
    id: &'a str,
    #[serde(rename = "CurrentPw")]
    current_pw: &'a str,
    #[serde(rename = "NewPw")]
    new_pw: &'a str,
}

#[derive(Debug, Serialize)]
struct EmbyCreateVirtualFolderRequest {
    #[serde(rename = "LibraryOptions")]
    library_options: EmbyLibraryOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
pub struct EmbyLibrary {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub library_type: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbyUser {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "LastActivityDate")]
    pub last_activity_date: Option<String>,
    #[serde(rename = "Policy", default)]
    pub policy: EmbyUserPolicy,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EmbyUserPolicy {
    #[serde(rename = "IsAdministrator")]
    pub is_administrator: Option<bool>,
    #[serde(rename = "IsDisabled")]
    pub is_disabled: Option<bool>,
    #[serde(rename = "RemoteClientBitrateLimit")]
    pub remote_client_bitrate_limit: Option<i64>,
    #[serde(rename = "SimultaneousStreamLimit")]
    pub simultaneous_stream_limit: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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

    pub async fn create_virtual_folder(
        &self,
        name: &str,
        collection_type: &str,
        path: &str,
        library_options: EmbyLibraryOptions,
    ) -> anyhow::Result<u16> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        if name.trim().is_empty() {
            bail!("library name is required for Emby virtual folder creation");
        }
        if collection_type.trim().is_empty() {
            bail!("collection_type is required for Emby virtual folder creation");
        }
        if path.trim().is_empty() {
            bail!("path is required for Emby virtual folder creation");
        }

        let url = format!("{}/Library/VirtualFolders", self.base_url);
        let response = self
            .http
            .post(url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("name", name.trim()),
                ("collectionType", collection_type.trim()),
                ("paths", path.trim()),
                ("refreshLibrary", "false"),
            ])
            .json(&EmbyCreateVirtualFolderRequest { library_options })
            .send()
            .await
            .context("failed to call Emby /Library/VirtualFolders")?;
        let status = response.status();
        response
            .error_for_status()
            .context("Emby /Library/VirtualFolders returned an error")?;
        Ok(status.as_u16())
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

    pub async fn series(&self, parent_id: &str, limit: usize) -> anyhow::Result<Vec<EmbyItem>> {
        if parent_id.trim().is_empty() {
            bail!("parent_id is required for Emby series listing");
        }
        let limit = limit.clamp(1, 100_000);
        let mut start = 0usize;
        let mut items = Vec::new();
        let mut total_record_count = None;

        while items.len() < limit {
            let page_limit = (limit - items.len()).min(1000);
            let page = self
                .items_page(parent_id, "Series", "ProviderIds", start, page_limit)
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

        Ok(items)
    }

    pub async fn library_items(
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
        let limit = limit.clamp(1, 30_000);
        let mut start = 0usize;
        let mut items = Vec::new();
        let mut total_record_count = None;

        while items.len() < limit {
            let page_limit = (limit - items.len()).min(1000);
            let page = self
                .items_page(
                    parent_id,
                    item_types,
                    "Path,ProductionYear,ProviderIds",
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

    pub async fn cleanup_items(
        &self,
        parent_id: &str,
        item_types: &str,
        limit: usize,
    ) -> anyhow::Result<EmbyCleanupItemsResult> {
        if parent_id.trim().is_empty() {
            bail!("parent_id is required for Emby cleanup item listing");
        }
        if item_types.trim().is_empty() {
            bail!("item_types is required for Emby cleanup item listing");
        }
        let limit = limit.clamp(1, 100_000);
        let mut start = 0usize;
        let mut items = Vec::new();
        let mut total_record_count = None;

        while items.len() < limit {
            let page_limit = (limit - items.len()).min(1000);
            let page = self
                .cleanup_items_page(parent_id, item_types, start, page_limit)
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

        Ok(EmbyCleanupItemsResult {
            truncated: total_record_count.is_some_and(|total| items.len() < total),
            items,
        })
    }

    pub async fn episodes(&self, series_id: &str) -> anyhow::Result<Vec<EmbyEpisode>> {
        if series_id.trim().is_empty() {
            bail!("series_id is required for Emby episode listing");
        }
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let path = format!("/Shows/{}/Episodes", urlencoding::encode(series_id.trim()));
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .http
            .get(url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("Fields", "ParentIndexNumber,IndexNumber,LocationType"),
                ("Limit", "6000"),
            ])
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .json::<EmbyEpisodesPage>()
            .await?
            .items)
    }

    pub async fn refresh_library(&self) -> anyhow::Result<u16> {
        self.post_empty("/Library/Refresh", &[]).await
    }

    pub async fn delete_item(&self, item_id: &str) -> anyhow::Result<u16> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby delete");
        }
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let path = format!("/Items/{}", urlencoding::encode(item_id.trim()));
        let url = format!("{}{}", self.base_url, path);
        let status = self
            .http
            .delete(url)
            .query(&[("api_key", self.api_key.as_str())])
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .status();
        Ok(status.as_u16())
    }

    pub async fn item_exists(&self, item_id: &str) -> anyhow::Result<bool> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby item lookup");
        }
        let page = self.items_by_ids_limited(item_id.trim(), "", 1).await?;
        Ok(!page.items.is_empty())
    }

    pub async fn notify_media_deleted(&self, path: &str) -> anyhow::Result<u16> {
        self.notify_media_updated([(path, "Deleted")]).await
    }

    pub async fn notify_media_updated<I, P, U>(&self, updates: I) -> anyhow::Result<u16>
    where
        I: IntoIterator<Item = (P, U)>,
        P: AsRef<str>,
        U: AsRef<str>,
    {
        let updates = updates
            .into_iter()
            .map(|(path, update_type)| {
                let path = path.as_ref().trim();
                let update_type = update_type.as_ref().trim();
                if path.is_empty() {
                    bail!("path is required for Emby media update notification");
                }
                if update_type.is_empty() {
                    bail!("update_type is required for Emby media update notification");
                }
                Ok(serde_json::json!({
                    "Path": path,
                    "UpdateType": update_type,
                }))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        if updates.is_empty() {
            bail!("updates must not be empty for Emby media update notification");
        }
        let body = serde_json::json!({ "Updates": updates });
        self.post_json_status("/Library/Media/Updated", &body).await
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

    pub async fn item(&self, item_id: &str, fields: &str) -> anyhow::Result<Option<EmbyItem>> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby item lookup");
        }
        let page = self.items_by_ids(item_id.trim(), fields).await?;
        Ok(page.items.into_iter().next())
    }

    pub async fn remote_search(
        &self,
        item_id: &str,
        name: &str,
        item_type: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<EmbyRemoteSearchCandidate>> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby remote search");
        }
        let name = name.trim();
        if name.is_empty() {
            bail!("name is required for Emby remote search");
        }
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let kind = remote_search_kind(item_type)?;
        let path = format!("/Items/RemoteSearch/{kind}");
        let body = RemoteSearchRequest {
            search_info: RemoteSearchInfo {
                name,
                provider_ids: BTreeMap::new(),
            },
            item_id: item_id.trim(),
            include_disabled_providers: true,
        };
        let mut candidates: Vec<EmbyRemoteSearchCandidate> = self.post_json(&path, &body).await?;
        candidates.truncate(limit.min(50));
        Ok(candidates)
    }

    pub async fn apply_remote_search(&self, item_id: &str, tmdb: &str) -> anyhow::Result<u16> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby remote search apply");
        }
        if tmdb.trim().is_empty() {
            bail!("tmdb is required for Emby remote search apply");
        }
        let path = format!(
            "/Items/RemoteSearch/Apply/{}",
            urlencoding::encode(item_id.trim())
        );
        let body = RemoteSearchApplyRequest {
            provider_ids: BTreeMap::from([("Tmdb", tmdb.trim())]),
        };
        self.post_json_status(&path, &body).await
    }

    pub async fn download_primary_image(
        &self,
        item_id: &str,
        image_url: &str,
    ) -> anyhow::Result<u16> {
        if item_id.trim().is_empty() {
            bail!("item_id is required for Emby primary image download");
        }
        if image_url.trim().is_empty() {
            bail!("image_url is required for Emby primary image download");
        }
        let path = format!(
            "/Items/{}/RemoteImages/Download",
            urlencoding::encode(item_id.trim())
        );
        self.post_empty(
            &path,
            &[("Type", "Primary"), ("ImageUrl", image_url.trim())],
        )
        .await
    }

    pub async fn users(&self) -> anyhow::Result<Vec<EmbyUser>> {
        self.get_json("/Users").await
    }

    pub async fn user(&self, user_id: &str) -> anyhow::Result<EmbyUser> {
        if user_id.trim().is_empty() {
            bail!("user_id is required for Emby user lookup");
        }
        let path = format!("/Users/{}", urlencoding::encode(user_id.trim()));
        self.get_json(&path).await
    }

    pub async fn update_user_policy(
        &self,
        user_id: &str,
        policy: &EmbyUserPolicy,
    ) -> anyhow::Result<u16> {
        if user_id.trim().is_empty() {
            bail!("user_id is required for Emby user policy update");
        }
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let path = format!("/Users/{}/Policy", urlencoding::encode(user_id.trim()));
        let url = format!("{}{}", self.base_url, path);
        let status = self
            .http
            .post(url)
            .query(&[("api_key", self.api_key.as_str())])
            .json(policy)
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .status();
        Ok(status.as_u16())
    }

    pub async fn create_user(
        &self,
        name: &str,
        password: Option<&str>,
    ) -> anyhow::Result<EmbyUser> {
        let name = name.trim();
        if name.is_empty() {
            bail!("user name is required for Emby user creation");
        }
        self.post_json_status("/Users/New", &EmbyCreateUserRequest { name })
            .await?;
        let created = self
            .users()
            .await?
            .into_iter()
            .find(|user| user.name == name)
            .with_context(|| format!("Emby user {name} was not found after creation"))?;
        if let Some(password) = password.filter(|value| !value.is_empty()) {
            self.set_user_password(&created.id, password).await?;
        }
        Ok(created)
    }

    pub async fn set_user_password(&self, user_id: &str, password: &str) -> anyhow::Result<u16> {
        if user_id.trim().is_empty() {
            bail!("user_id is required for Emby password update");
        }
        let path = format!("/Users/{}/Password", urlencoding::encode(user_id.trim()));
        self.post_json_status(
            &path,
            &EmbySetPasswordRequest {
                id: user_id.trim(),
                current_pw: "",
                new_pw: password,
            },
        )
        .await
    }

    pub async fn delete_user(&self, user_id: &str) -> anyhow::Result<u16> {
        if user_id.trim().is_empty() {
            bail!("user_id is required for Emby user deletion");
        }
        let path = format!("/Users/{}", urlencoding::encode(user_id.trim()));
        self.delete_status(&path).await
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

    async fn get_json<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .http
            .get(url)
            .query(&[("api_key", self.api_key.as_str())])
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .json()
            .await?)
    }

    async fn post_json<T, B>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize + ?Sized,
    {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .http
            .post(url)
            .query(&[("api_key", self.api_key.as_str())])
            .json(body)
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .json()
            .await?)
    }

    async fn post_json_status<B>(&self, path: &str, body: &B) -> anyhow::Result<u16>
    where
        B: Serialize + ?Sized,
    {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}{}", self.base_url, path);
        let status = self
            .http
            .post(url)
            .query(&[("api_key", self.api_key.as_str())])
            .json(body)
            .send()
            .await
            .with_context(|| format!("failed to call Emby {path}"))?
            .error_for_status()
            .with_context(|| format!("Emby {path} returned an error"))?
            .status();
        Ok(status.as_u16())
    }

    async fn delete_status(&self, path: &str) -> anyhow::Result<u16> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}{}", self.base_url, path);
        let status = self
            .http
            .delete(url)
            .query(&[("api_key", self.api_key.as_str())])
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

    async fn cleanup_items_page(
        &self,
        parent_id: &str,
        item_types: &str,
        start_index: usize,
        limit: usize,
    ) -> anyhow::Result<EmbyCleanupItemsPage> {
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
                (
                    "Fields",
                    "Path,ProductionYear,ProviderIds,ImageTags,CommunityRating,CriticRating,UserData",
                ),
                ("SortBy", "SortName"),
                ("StartIndex", &start_index.to_string()),
                ("Limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("failed to call Emby /Items for cleanup suggestions")?
            .error_for_status()
            .context("Emby /Items returned an error for cleanup suggestions")?
            .json()
            .await?)
    }

    async fn items_by_ids(&self, ids: &str, fields: &str) -> anyhow::Result<EmbyItemsPage> {
        self.items_by_ids_limited(ids, fields, 100).await
    }

    async fn items_by_ids_limited(
        &self,
        ids: &str,
        fields: &str,
        limit: usize,
    ) -> anyhow::Result<EmbyItemsPage> {
        if !self.has_api_key() {
            bail!("api_key is not configured for Emby requests");
        }
        let url = format!("{}/Items", self.base_url);
        let limit = limit.clamp(1, 100_000).to_string();
        Ok(self
            .http
            .get(url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("Ids", ids),
                ("Fields", fields),
                ("Limit", limit.as_str()),
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

impl EmbyCleanupItem {
    pub fn rating(&self) -> Option<f64> {
        self.community_rating
            .or(self
                .user_data
                .as_ref()
                .and_then(|user_data| user_data.rating))
            .or_else(|| self.critic_rating.map(|rating| rating / 10.0))
            .filter(|rating| rating.is_finite() && *rating >= 0.0)
    }

    pub fn has_provider_id(&self, key: &str) -> bool {
        self.provider_ids
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .is_some_and(|(_, value)| match value {
                Value::String(s) => !s.trim().is_empty(),
                Value::Null => false,
                _ => true,
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

fn remote_search_kind(item_type: &str) -> anyhow::Result<&'static str> {
    match item_type.trim().to_ascii_lowercase().as_str() {
        "series" | "tvshow" | "tvshows" | "show" => Ok("Series"),
        "movie" | "movies" => Ok("Movie"),
        other => bail!("unsupported Emby remote search type: {other}"),
    }
}
