# Smart Action Engine 智能动作引擎开发计划

## 1. 背景与目标

当前 Rust 版已经具备一批“智能化雏形”：

- 首页 `/api/v2/dashboard/smart-actions` 可以聚合无海报、无评分、去重、追更、任务异常等待办。
- 115 一条龙 `/api/v2/wizard/add-new` 已经串起转存、STRM、Emby 扫描、海报检测/自动修复、去重检测、旧版本自动处理和结果检查。
- 找资源 `/api/v2/catalog/remote-search` 已经能叠加 Emby 当前库上下文和资源推荐分。
- 追更 `/api/v2/zhuigeng/workbench` 已经能判断更新、归档、异常，并能对接找资源和更新执行。
- 去重界面已经具备智能保留建议、批量执行、115 删除协同。
- 任务中心已经开始用结构化结果替代直接暴露 JSON。

这些能力目前仍然分散在不同业务模块里。下一阶段的目标不是继续加独立功能，而是把它们抽象成统一的 Smart Action Engine：

- 系统围绕“媒体对象”自动采集证据。
- 系统解释“发现了什么问题、为什么推荐这个动作”。
- 系统区分“可自动执行”和“需要人工确认”的动作。
- 系统以统一方式执行、记录、验证、失败诊断和回滚。
- 前端从“功能入口集合”升级为“智能动作工作台”。

上线目标：

- 首页智能下一步升级为智能动作中心摘要。
- 新增独立 Smart Actions 工作台，支持按媒体、风险、动作类型聚合。
- 115 转存、找资源、追更、去重、海报修复、扫描、清理都可以产出统一 SmartAction。
- 高置信安全动作支持批量自动执行；高风险动作必须二次确认。
- 所有动作执行后必须有验收结果，不能只显示“任务完成”。

## 2. 核心产品原则

1. **以媒体对象为中心**
   不再让用户在功能之间跳来跳去。系统先告诉用户某部电影/电视剧当前状态，再给出可执行动作。

2. **推荐必须可解释**
   每个动作都要有证据、规则、评分和风险说明。用户要知道系统为什么建议转存、替换、归档、修海报或跳过。

3. **安全优先**
   删除、替换、移动、归档、115 删除、Emby 删除都必须有预检、undo 或审计记录。没有足够证据时降级为人工确认。

4. **执行后必须闭环**
   转存成功不等于业务成功。必须继续验证 STRM、Emby 条目、海报、TMDb、重复资源和旧资源清理结果。

5. **尽量复用现有业务实现**
   Smart Action Engine 是编排层，不重复实现 115、Emby、去重、追更、海报修复逻辑。它调用现有 typed functions / endpoints / task pipeline。

## 3. SmartAction 数据模型

新增后端模块建议：`server/src/smart_actions.rs`。

核心结构：

```rust
pub struct SmartAction {
    pub id: Uuid,
    pub action_type: SmartActionType,
    pub status: SmartActionStatus,
    pub subject: SmartSubject,
    pub title: String,
    pub summary: String,
    pub recommendation: SmartRecommendation,
    pub evidence: Vec<SmartEvidence>,
    pub plan: SmartExecutionPlan,
    pub risk: SmartRisk,
    pub policy: SmartPolicyDecision,
    pub verification: SmartVerificationPlan,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### 3.1 Subject 媒体对象

```rust
pub enum SmartSubjectKind {
    Movie,
    Series,
    Season,
    Episode,
    Library,
    Task,
    System,
    Unknown,
}

pub struct SmartSubject {
    pub kind: SmartSubjectKind,
    pub name: String,
    pub year: Option<i32>,
    pub tmdb: Option<String>,
    pub emby_id: Option<String>,
    pub lib: Option<String>,
    pub folder: Option<String>,
    pub strm_path: Option<String>,
    pub cd_path: Option<String>,
}
```

### 3.2 ActionType 动作类型

首批动作类型：

- `transfer_add_new`：115/离线资源一条龙转存。
- `transfer_update_series`：追更剧找资源并更新。
- `dedup_remove_old`：删除重复旧版本。
- `dedup_review`：重复资源需要人工确认。
- `poster_fix`：海报/TMDb 自动修复。
- `metadata_refresh`：刷新 Emby 元数据。
- `library_scan`：刷新库或单条目扫描。
- `archive_series`：完结剧归档到正式库。
- `cleanup_empty_folder`：清理空目录/孤儿 STRM。
- `task_retry_or_diagnose`：失败任务诊断/重试。

### 3.3 Recommendation 推荐结果

```rust
pub struct SmartRecommendation {
    pub score: i32,
    pub confidence: SmartConfidence,
    pub primary_action: String,
    pub reasons: Vec<String>,
    pub alternatives: Vec<SmartAlternative>,
}

pub enum SmartConfidence {
    High,
    Medium,
    Low,
}
```

评分建议：

- `>= 85`：高置信，可自动执行，但删除类仍需要策略允许。
- `60..84`：中置信，默认人工确认。
- `< 60`：低置信，只给建议和诊断，不自动执行。

### 3.4 Evidence 证据

证据必须保留来源，方便前端展示和后续审计：

```rust
pub struct SmartEvidence {
    pub source: SmartEvidenceSource,
    pub label: String,
    pub value: serde_json::Value,
    pub weight: i32,
    pub collected_at: DateTime<Utc>,
}
```

证据来源：

- `emby_item`
- `emby_episodes`
- `strm_scan`
- `cloud_drive_path`
- `c115_resource`
- `catalog_candidate`
- `tmdb_metadata`
- `poster_detection`
- `dedup_analysis`
- `task_history`
- `undo_log`
- `system_health`

### 3.5 Risk 风险模型

```rust
pub struct SmartRisk {
    pub level: SmartRiskLevel,
    pub destructive: bool,
    pub touches_emby: bool,
    pub touches_disk: bool,
    pub touches_c115: bool,
    pub requires_confirm_text: Option<String>,
    pub warnings: Vec<String>,
}
```

风险规则：

- 只读诊断：`low`
- 刷新 Emby / 修海报：`medium`
- 移动目录 / 归档：`high`
- 删除 STRM / 删除 CloudDrive / 删除 115：`critical`

### 3.6 ExecutionPlan 执行计划

```rust
pub struct SmartExecutionPlan {
    pub steps: Vec<SmartExecutionStep>,
    pub estimated_seconds: Option<i64>,
    pub concurrency_key: Option<String>,
    pub can_cancel: bool,
}

pub struct SmartExecutionStep {
    pub key: String,
    pub title: String,
    pub executor: SmartExecutorKind,
    pub params: serde_json::Value,
    pub rollback: Option<SmartRollbackStep>,
}
```

执行器不直接拼外部 URL，必须调用现有 typed module：

- `wizard::create_add_new_task`
- `zhuigeng::zhuigeng_update_execute_for_state`
- `zhuigeng::zhuigeng_archive_execute_for_state`
- `dedup::execute_dedup_batch`
- `dedup::auto_all`
- `posters::apply_poster_match` / `fix-batch`
- `media_fs` 管理/移动/删除能力
- `emby` typed client
- `tasks` 任务持久化
- `undo` 审计/回滚

## 4. 数据库设计

新增 migrations：

```sql
CREATE TABLE smart_action_runs (
  id uuid PRIMARY KEY,
  action_type text NOT NULL,
  status text NOT NULL,
  subject jsonb NOT NULL,
  title text NOT NULL,
  summary text NOT NULL,
  recommendation jsonb NOT NULL,
  evidence jsonb NOT NULL,
  plan jsonb NOT NULL,
  risk jsonb NOT NULL,
  policy jsonb NOT NULL,
  verification jsonb NOT NULL,
  task_id uuid NULL REFERENCES task_runs(id),
  result jsonb NULL,
  error text NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NULL
);

CREATE INDEX smart_action_runs_status_idx ON smart_action_runs(status, updated_at DESC);
CREATE INDEX smart_action_runs_action_type_idx ON smart_action_runs(action_type, updated_at DESC);
CREATE INDEX smart_action_runs_subject_gin_idx ON smart_action_runs USING gin(subject);
CREATE INDEX smart_action_runs_evidence_gin_idx ON smart_action_runs USING gin(evidence);

CREATE TABLE smart_action_policies (
  key text PRIMARY KEY,
  enabled boolean NOT NULL DEFAULT true,
  mode text NOT NULL DEFAULT 'confirm',
  max_risk text NOT NULL DEFAULT 'medium',
  params jsonb NOT NULL DEFAULT '{}',
  updated_at timestamptz NOT NULL DEFAULT now()
);
```

状态：

- `suggested`
- `confirmed`
- `queued`
- `running`
- `verifying`
- `done`
- `partial`
- `failed`
- `cancelled`
- `dismissed`

保留策略：

- 最近 30 天动作保留完整证据。
- 已完成低风险动作可 90 天后压缩证据。
- 删除/移动/115 删除相关动作永久保留审计摘要。

## 5. API 设计

新增路径统一在 `/api/v2/smart-actions/*`。

### 5.1 查询

- `GET /api/v2/smart-actions/summary`
  首页摘要：待确认、高置信可执行、失败、执行中。

- `GET /api/v2/smart-actions`
  动作列表，支持筛选：
  - `status`
  - `action_type`
  - `risk`
  - `subject_kind`
  - `lib`
  - `q`
  - `limit`
  - `offset`

- `GET /api/v2/smart-actions/{id}`
  动作详情：证据、规则命中、执行计划、验证计划、历史结果。

### 5.2 生成建议

- `POST /api/v2/smart-actions/inspect`
  对指定对象即时生成动作建议，不一定持久化。

- `POST /api/v2/smart-actions/refresh`
  后台刷新全库动作建议，返回 TaskRun。

- `POST /api/v2/smart-actions/from-task/{task_id}`
  对失败或半成功任务生成诊断动作。

### 5.3 执行

- `POST /api/v2/smart-actions/{id}/execute`
  执行单个动作，返回 TaskRun。

- `POST /api/v2/smart-actions/execute-batch`
  批量执行动作。只允许符合策略的动作进入自动执行。

- `POST /api/v2/smart-actions/{id}/dismiss`
  用户忽略建议，可带原因。

- `POST /api/v2/smart-actions/{id}/verify`
  重新验证动作结果。

### 5.4 策略

- `GET /api/v2/smart-actions/policies`
- `PUT /api/v2/smart-actions/policies/{key}`

## 6. 证据采集器设计

新增 trait：

```rust
#[async_trait]
pub trait SmartEvidenceCollector {
    async fn collect(&self, ctx: &SmartCollectContext) -> AppResult<Vec<SmartEvidence>>;
}
```

首批采集器：

1. **EmbyCollector**
   - 读取库、Item、ProviderIds、Path、ImageTags、UserData、集数。
   - 识别没有 TMDbId、没有海报、重复 ProviderId、路径不规范。

2. **StrmCollector**
   - 检查 STRM 是否存在、路径是否指向 CloudDrive、孤儿/空目录。
   - 保持 CloudDrive 单路执行策略，不并发扫 mount。

3. **CatalogCollector**
   - 调用本地 catalog 和远程资源 API。
   - 计算候选资源集数、类型、是否整包、是否可能覆盖缺集。

4. **TmdbCollector**
   - 读取剧集状态、last/next episode、完结状态。
   - TMDb 未配置时降级，不阻断非 TMDb 动作。

5. **DedupCollector**
   - 复用 `dedup::analyze_duplicate_groups`。
   - 聚合文件夹重复、Emby ProviderId 重复、跨库重复。

6. **TaskHistoryCollector**
   - 读取最近失败、partial、cancelled、interrupted 任务。
   - 从任务结果里识别阶段性失败，例如转存成功但扫描失败、海报修复失败。

7. **SystemCollector**
   - 检查 Emby 可达性、磁盘、容器健康、路径映射。
   - 只生成系统级动作，不直接执行删除类动作。

## 7. 规则引擎设计

新增 trait：

```rust
pub trait SmartRule {
    fn id(&self) -> &'static str;
    fn evaluate(&self, facts: &SmartFacts) -> Vec<SmartActionDraft>;
}
```

规则分组：

### 7.1 转存与更新规则

- 新资源候选覆盖本地缺集，且同 TMDb，推荐 `transfer_update_series`。
- 转存成功但 Emby 未出现，推荐 `library_scan` 或路径诊断。
- 115 候选与库内已有资源完全重复，推荐跳过或人工确认。
- 候选资源集数明显少于本地，不推荐替换，只允许补充或忽略。

### 7.2 去重规则

- 同 TMDb 多个文件夹，目标库/正式库优先保留。
- 追更库旧目录和新目录并存，新目录集数更多且路径更新，推荐删除旧目录。
- 同一个 Emby ProviderId 出现多个 Item，若一个路径不存在，推荐删除不存在项。
- 无 TMDb 或 TMDb 冲突，降级为 `dedup_review`。

### 7.3 海报与元数据规则

- 有 TMDbId 且无 Primary 图片，推荐 `poster_fix`。
- 文件夹声明 TMDb 与 Emby ProviderId 不一致，推荐人工确认或修复。
- 一条龙转存后无 TMDbId，但资源名可搜索到高置信候选，推荐自动 Apply + Refresh。
- TMDb 未配置时生成配置诊断动作，不让任务假完成。

### 7.4 追更规则

- TMDb continuing 且本地落后，推荐 `transfer_update_series`。
- TMDb ended 且本地已达到 last episode，推荐 `archive_series`。
- TMDb ended 但本地仍缺集，推荐先找资源补齐，再归档。
- TMDb 状态异常或缺 ProviderId，推荐元数据修复。

### 7.5 任务诊断规则

- 任务 `done` 但结果里有 `check.stage_error_count > 0`，标为 `partial` 动作。
- 一条龙任务 `transfer.ok=true` 但 `scan.ok=false`，推荐重新扫描/路径诊断。
- `poster_auto_fix.error_count > 0`，推荐打开海报修复。
- `auto_resolve.error_count > 0`，推荐去重人工确认。

## 8. 策略与自动化边界

默认策略：

| 动作 | 默认模式 | 最大自动风险 | 说明 |
| --- | --- | --- | --- |
| poster_fix | auto | medium | 有 TMDbId 且候选高置信可自动 |
| metadata_refresh | auto | medium | 只触发 Emby 刷新 |
| library_scan | auto | medium | 只触发扫描 |
| transfer_update_series | confirm | high | 涉及转存和旧资源处理 |
| dedup_remove_old | confirm | critical | 涉及 Emby/磁盘/115 删除 |
| archive_series | confirm | high | 涉及移动和库刷新 |
| cleanup_empty_folder | confirm | high | 涉及磁盘删除 |
| task_retry_or_diagnose | confirm | medium | 避免循环重试 |

可配置项：

- 是否允许高置信海报自动修复。
- 是否允许转存后自动删除同剧旧目录。
- 是否允许去重同步删除 115 资源。
- 是否允许完结剧批量归档。
- 每轮最多自动执行多少动作。
- CloudDrive 类动作全局串行，保持现有 115 风控约束。

## 9. 执行器与任务中心整合

Smart Action 执行必须生成 TaskRun：

- `kind = smart_action_execute`
- `source = smart_actions`
- `params` 包含 action id、action type、subject 摘要。
- `result` 使用统一结构：

```json
{
  "action_id": "...",
  "action_type": "transfer_update_series",
  "subject": {},
  "steps": [
    {
      "key": "transfer",
      "status": "done",
      "message": "转存 1 项成功",
      "result": {}
    }
  ],
  "verification": {
    "status": "done",
    "checks": []
  },
  "next_actions": []
}
```

任务中心展示：

- 默认展示业务摘要。
- 每个 step 有状态、耗时、结果。
- 失败时展示“下一步按钮”：
  - 重试当前步骤
  - 打开海报修复
  - 打开去重人工确认
  - 打开路径映射设置
  - 查看技术详情 JSON

## 10. 前端设计

新增主入口：`智能动作`。

### 10.1 首页摘要

替换当前简单卡片为四组：

- 可一键执行
- 需要确认
- 执行中
- 失败/半成功

点击进入工作台，并保留现有跳转到具体 tab 的能力。

### 10.2 Smart Action Workbench

布局：

- 顶部过滤栏：动作类型、风险、状态、库、关键词。
- 左侧动作列表：按媒体对象聚合。
- 右侧详情 Drawer：
  - 当前状态
  - 推荐动作
  - 证据
  - 风险
  - 执行步骤
  - 验收条件
  - 相关历史任务

### 10.3 批量确认

支持三类批量：

- 批量执行高置信低/中风险动作。
- 批量确认归档候选。
- 批量确认旧版本删除候选。

删除/替换/115 删除必须使用 `ConfirmDanger`，显示受影响对象数量和示例路径。

### 10.4 与现有页面融合

每个页面都能嵌入当前对象的 SmartAction：

- 找资源：显示“库内现状 + 推荐动作”。
- 追更：每个剧卡显示动作建议，更新/归档走 SmartAction。
- 去重：保留项和删除项由 SmartAction 规则解释。
- 海报修复：候选项可生成 SmartAction。
- 任务中心：失败任务可生成诊断动作。

## 11. OpenAPI 与类型生成

要求：

- 所有 SmartAction DTO 由 Rust `utoipa::ToSchema` 生成。
- 前端只使用 `web/src/api/openapi.d.ts` 类型。
- CI 校验 `npm run openapi:types` 后无 diff。

新增 OpenAPI 测试：

- `/api/v2/smart-actions`
- `/api/v2/smart-actions/summary`
- `/api/v2/smart-actions/refresh`
- `/api/v2/smart-actions/{id}/execute`
- `/api/v2/smart-actions/execute-batch`
- `/api/v2/smart-actions/{id}/verify`
- `SmartAction`
- `SmartEvidence`
- `SmartExecutionPlan`
- `SmartRisk`
- `SmartPolicy`

## 12. 实施阶段

### Phase 1：模型与只读建议

目标：把当前 `/dashboard/smart-actions` 升级为真正 SmartAction 列表，但不执行写操作。

任务：

- 新增 `server/src/smart_actions.rs`。
- 新增 DB migration：`smart_action_runs`、`smart_action_policies`。
- 实现 SmartAction DTO、OpenAPI、路由。
- 复用 dashboard 现有聚合逻辑，生成第一批只读动作。
- 前端新增“智能动作”入口和工作台只读列表。

验收：

- 首页仍可显示智能下一步。
- `/api/v2/smart-actions` 返回结构化动作。
- 动作详情能展示证据和推荐原因。
- 不执行任何写操作。

### Phase 2：证据采集器与规则引擎

目标：从聚合待办升级为按媒体对象推理。

任务：

- 实现 EmbyCollector、TaskHistoryCollector、DedupCollector、PosterCollector。
- 实现基础 SmartRule trait。
- 实现无海报、重复、失败任务、追更归档候选规则。
- 动作持久化并支持过期刷新。

验收：

- 同一媒体对象的多个问题能聚合在一张详情里。
- 每个动作有 score、confidence、risk、evidence。
- 单元测试覆盖评分和风险降级。

### Phase 3：执行器与验证闭环

目标：让低/中风险动作可执行，结果可验证。

任务：

- 实现 `execute_action`。
- 接入 poster_fix、metadata_refresh、library_scan。
- 实现统一 TaskRun result。
- 实现 `verify_action`。
- 任务中心展示 SmartAction step 结果。

验收：

- 海报修复动作可从工作台执行。
- 执行后状态从 `queued/running/verifying` 到 `done/partial/failed`。
- 失败时生成下一步诊断动作。

### Phase 4：一条龙转存与追更动作化

目标：把找资源和追更接入 SmartAction。

任务：

- `catalog/remote-search` 结果可生成 `transfer_add_new` 动作。
- `zhuigeng/workbench` 行可生成 `transfer_update_series`、`archive_series` 动作。
- `wizard/add-new` 执行结果反写 SmartAction。
- 一条龙半成功结果生成后续动作。

验收：

- 找资源搜索后能看到“建议转存/跳过/替换旧版”。
- 追更页面批量“智能找资源 -> 更新推荐”走 SmartAction。
- 转存后若 Emby 未出现，动作状态为 partial 并给出扫描/诊断动作。

### Phase 5：高风险动作与批量确认

目标：安全接入去重、旧资源删除、115 删除、归档。

任务：

- 接入 `dedup_remove_old`。
- 接入 `archive_series`。
- 接入策略配置和 ConfirmDanger。
- 为删除/移动/115 删除写入 undo/audit。
- 批量执行时按风险拆分：自动、需确认、禁止。

验收：

- 去重推荐保留项有可解释评分。
- 删除旧资源前明确列出 Emby、STRM、CloudDrive、115 影响。
- 失败时不误报 done。
- undo/audit 可追踪。

### Phase 6：系统化自动巡检

目标：让引擎定期产出新建议。

任务：

- 新增 schedule kind：`smart_actions_refresh`。
- 每日或每 6 小时刷新动作建议。
- 高置信低风险动作可按策略自动执行。
- 首页显示趋势：新增、已完成、失败、忽略。

验收：

- 定时刷新不与 CloudDrive 任务冲突。
- 重启后 running 动作能恢复为 interrupted/failed。
- 可配置自动化边界。

## 13. 测试计划

### Rust 单元测试

- SmartAction score 排序。
- 风险等级计算。
- TMDb 未配置降级。
- 重复资源规则。
- 完结剧归档规则。
- 任务 partial 识别。
- 策略阻止高风险自动执行。

### Rust 集成测试

- Postgres migration。
- fake Emby：无海报、重复 ProviderId、集数落后、完结状态。
- fake catalog：候选资源覆盖缺集。
- fake 115：转存成功/失败。
- SmartAction execute -> TaskRun -> verify。

### 前端测试

- 工作台加载与过滤。
- 动作详情 Drawer。
- 高风险 ConfirmDanger。
- 批量执行按钮状态。
- 任务中心 SmartAction result 展示。
- 首页摘要跳转。

### NAS 验收

- 首页智能动作摘要。
- 搜一部库内已有剧，展示库内现状和资源推荐。
- 追更剧落后，生成更新动作。
- 完结剧生成归档动作。
- 无海报条目生成海报修复动作。
- 重复资源生成保留/删除建议。
- 执行一个低风险动作并验证 done。
- 执行一个需要确认动作并检查 audit/undo。

## 14. 风险与控制

### 14.1 误删风险

控制：

- 删除类动作默认不自动执行。
- 必须展示路径、Emby item、115 影响。
- 必须写 undo/audit。
- 删除 Emby 仍保持“先 Emby DELETE，再动磁盘”的既有规则。

### 14.2 115 风控风险

控制：

- CloudDrive 读取类动作继续使用单路执行。
- 批量动作中涉及 CloudDrive 的步骤串行。
- 每个步骤保留最小间隔。

### 14.3 推荐误判

控制：

- 低置信动作只提示，不执行。
- 缺 TMDb 或 TMDb 冲突时降级人工确认。
- 多证据冲突时不自动删除。

### 14.4 任务状态误报

控制：

- done 只表示执行器完成，SmartAction 还必须通过 verification 才算 done。
- verification 失败标 partial。
- 前端任务中心必须展示 partial 原因。

## 15. 首批交付边界

第一版不要贪多。建议先交付：

- SmartAction 模型和 API。
- 首页摘要接入新 API。
- 工作台只读列表和详情。
- 无海报、无评分、去重、追更、失败任务五类建议。
- poster_fix、library_scan、metadata_refresh 三类低/中风险执行。
- 任务中心 SmartAction result 展示。

暂缓：

- 完全自动删除旧资源。
- 大规模批量归档。
- 跨库复杂替换。
- 自动修改 TMDb 错绑。

这些在第二轮接入，等规则和证据展示稳定后再放开。

## 16. 推荐提交节奏

建议拆成 6 个 PR/commit：

1. `feat(smart-actions): add action model and readonly API`
2. `feat(smart-actions): add evidence collectors and rules`
3. `feat(web): add smart action workbench`
4. `feat(smart-actions): execute low-risk actions`
5. `feat(smart-actions): connect catalog and zhuigeng workflows`
6. `feat(smart-actions): add high-risk confirmation and policies`

每个 commit 都要求：

- `cargo test --workspace`
- `cargo clippy --workspace --lib --bins -- -D warnings`
- `cd web && npm test -- --run`
- `cd web && npm run build`
- `npm run openapi:types` 后无 diff

## 17. 最终验收标准

项目从“功能集合”进化到“智能动作引擎”的标志：

- 用户打开首页能看到系统推荐下一步，而不是自己判断点哪个 tab。
- 用户点进动作能看到证据、推荐理由、风险和执行计划。
- 用户执行动作后能看到每一步是否真的成功。
- 转存、追更、去重、海报修复、归档不再割裂。
- 失败任务能自动转化为可操作的诊断动作。
- 删除和替换类操作不再“黑盒完成”，而是有确认、有审计、有验证。
