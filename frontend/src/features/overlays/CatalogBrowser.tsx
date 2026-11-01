/**
 * 模型目录浏览器。
 *
 * 从嵌入的 models.dev 快照里选择提供商与模型，导入为普通用户配置。
 * 应用没有任何内置提供商，也没有“推荐”排序；这里只按名称与标识搜索。
 */
import { useEffect, useMemo, useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import type { CatalogMeta, CatalogModelView, CatalogProviderView, ProviderView } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";

/**
 * 目录浏览器组件。
 *
 * props：
 * - meta：目录版本信息，用于页脚展示；
 * - onCancel：关闭浏览器；
 * - onImported：导入成功后的回调，参数是后端返回的完整提供商视图。
 */
export function CatalogBrowser({
  meta,
  onCancel,
  onImported,
}: {
  meta: CatalogMeta | null;
  onCancel: () => void;
  onImported: (provider: ProviderView) => void;
}) {
  const toast = useUiStore((state) => state.toast);
  const [providers, setProviders] = useState<CatalogProviderView[]>([]);
  const [providersLoading, setProvidersLoading] = useState(true);
  const [providerQuery, setProviderQuery] = useState("");
  const [selectedProvider, setSelectedProvider] = useState<CatalogProviderView | null>(null);

  const [models, setModels] = useState<CatalogModelView[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelQuery, setModelQuery] = useState("");
  const [requireTools, setRequireTools] = useState(true);
  const [requireVision, setRequireVision] = useState(false);
  const [showDeprecated, setShowDeprecated] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // 是否要求 API Key；端点、名称与协议直接采用目录数据，导入后仍可编辑。
  const [keyRequired, setKeyRequired] = useState(true);
  const [error, setError] = useState("");
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setProvidersLoading(true);
    api.providers
      .catalogProviders("")
      .then((items) => {
        if (!cancelled) {
          setProviders(items);
        }
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(String(caught));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setProvidersLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 切换提供商时重新读取模型，并重置选择与过滤器。
  useEffect(() => {
    if (!selectedProvider) {
      setModels([]);
      return;
    }
    let cancelled = false;
    setModelsLoading(true);
    setSelected(new Set());
    setModelQuery("");
    setKeyRequired(true);
    setError("");
    api.providers
      .catalogModels(selectedProvider.id)
      .then((items) => {
        if (!cancelled) {
          setModels(items);
        }
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(String(caught));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setModelsLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedProvider]);

  // 提供商搜索在本地完成，216 条数据不需要每次请求后端。
  const visibleProviders = useMemo(() => {
    const query = providerQuery.trim().toLowerCase();
    if (!query) {
      return providers;
    }
    return providers.filter(
      (provider) =>
        provider.id.toLowerCase().includes(query) || provider.name.toLowerCase().includes(query),
    );
  }, [providers, providerQuery]);

  // 模型筛选：关键字、工具、图片与弃用状态。
  const visibleModels = useMemo(() => {
    const query = modelQuery.trim().toLowerCase();
    return models.filter((item) => {
      if (query && !item.id.toLowerCase().includes(query) && !item.name.toLowerCase().includes(query)) {
        return false;
      }
      if (requireTools && !item.caps.tools) {
        return false;
      }
      if (requireVision && !item.caps.vision) {
        return false;
      }
      if (!showDeprecated && item.status === "deprecated") {
        return false;
      }
      return true;
    });
  }, [models, modelQuery, requireTools, requireVision, showDeprecated]);

  /** 切换单个模型的选中状态。 */
  function toggleModel(modelID: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(modelID)) {
        next.delete(modelID);
      } else {
        next.add(modelID);
      }
      return next;
    });
  }

  /** 选中当前筛选结果。 */
  function selectVisible() {
    setSelected((current) => {
      const next = new Set(current);
      for (const item of visibleModels) {
        next.add(item.id);
      }
      return next;
    });
  }

  /** 清空选择。 */
  function clearSelection() {
    setSelected(new Set());
  }

  /** 提交导入。 */
  async function importSelected() {
    if (!selectedProvider) {
      return;
    }
    if (selected.size === 0) {
      setError("至少选择一个模型");
      return;
    }
    setImporting(true);
    setError("");
    try {
      const saved = await api.providers.importFromCatalog({
        providerId: selectedProvider.id,
        modelIds: [...selected],
        all: false,
        keyRequired,
      });
      toast({
        title: "已导入提供商",
        description: `${saved.name} · ${saved.models.length} 个模型`,
        tone: "ok",
      });
      onImported(saved);
    } catch (caught) {
      setError(String(caught));
    } finally {
      setImporting(false);
    }
  }

  return (
    <div className="provider-editor catalog-browser">
      <div className="provider-editor-head">
        <Icon name="globe" />
        <h3>从模型目录添加</h3>
        <span className="spacer" />
        <button type="button" className="icon-btn sm" aria-label="关闭模型目录" onClick={onCancel}>
          <Icon name="x" />
        </button>
      </div>

      <div className="catalog-panes">
        <div className="catalog-provider-pane">
          <label className="field">
            <span>搜索提供商</span>
            <div className="key-input">
              <input
                className="text-input wide"
                value={providerQuery}
                placeholder="名称或 ID，例如 openrouter"
                onChange={(event) => setProviderQuery(event.target.value)}
              />
              <Icon name="search" className="ico" />
            </div>
          </label>

          <div className="catalog-provider-list">
            {providersLoading ? <div className="sec-desc">正在读取目录…</div> : null}
            {!providersLoading && visibleProviders.length === 0 ? (
              <div className="sec-desc">没有匹配的提供商</div>
            ) : null}
            {visibleProviders.map((provider) => (
              <button
                key={provider.id}
                type="button"
                className={cn(
                  "catalog-provider-item",
                  selectedProvider?.id === provider.id && "is-active",
                  provider.added && "is-added",
                )}
                onClick={() => setSelectedProvider(provider)}
              >
                <span className="catalog-provider-logo">{provider.name.slice(0, 2)}</span>
                <span className="catalog-provider-main">
                  <span className="catalog-provider-name">{provider.name}</span>
                  <span className="catalog-provider-meta">{provider.modelCount} 个模型</span>
                </span>
                {provider.added ? <span className="badge neutral">已添加</span> : null}
                <Icon name="chevron-right" className="ico" />
              </button>
            ))}
          </div>
        </div>

        <div className="catalog-model-pane">
          {!selectedProvider ? (
            <div className="panel-empty">
              <Icon name="globe" size={20} />
              <div>选择一个提供商</div>
              <div className="panel-empty-hint">左侧列表只来自 models.dev，不包含任何内置或推荐条目。</div>
            </div>
          ) : (
            <>
              <div className="catalog-model-head">
                <div>
                  <div className="catalog-model-title">{selectedProvider.name}</div>
                  <div className="sec-desc">
                    {selectedProvider.id} · {selectedProvider.modelCount} 个模型
                    {selectedProvider.doc ? (
                      <>
                        {" · "}
                        <button
                          type="button"
                          className="link-btn"
                          onClick={() => void api.app.openExternal(selectedProvider.doc!)}
                        >
                          文档
                        </button>
                      </>
                    ) : null}
                  </div>
                </div>
                <span className="spacer" />
                {selectedProvider.added ? <span className="badge neutral">已添加，请直接编辑</span> : null}
              </div>

              <div className="catalog-import-summary">
                <span className="mono">{selectedProvider.endpoint}</span>
                <span className="spacer" />
                <label className="cap-check">
                  <input
                    type="checkbox"
                    checked={keyRequired}
                    onChange={(event) => setKeyRequired(event.target.checked)}
                  />
                  需要 API Key（本地端点可以关闭）
                </label>
              </div>

              <div className="catalog-filters">
                <div className="key-input catalog-search">
                  <input
                    className="text-input"
                    value={modelQuery}
                    placeholder="搜索模型 ID 或名称"
                    onChange={(event) => setModelQuery(event.target.value)}
                  />
                  <Icon name="search" className="ico" />
                </div>
                <label className="cap-check">
                  <input
                    type="checkbox"
                    checked={requireTools}
                    onChange={(event) => setRequireTools(event.target.checked)}
                  />
                  工具
                </label>
                <label className="cap-check">
                  <input
                    type="checkbox"
                    checked={requireVision}
                    onChange={(event) => setRequireVision(event.target.checked)}
                  />
                  图片
                </label>
                <label className="cap-check">
                  <input
                    type="checkbox"
                    checked={showDeprecated}
                    onChange={(event) => setShowDeprecated(event.target.checked)}
                  />
                  显示弃用
                </label>
                <span className="spacer" />
                <button type="button" className="btn sm ghost" onClick={selectVisible}>
                  全选当前结果
                </button>
                <button type="button" className="btn sm ghost" onClick={clearSelection}>
                  清空
                </button>
              </div>

              <div className="catalog-model-table-wrap">
                {modelsLoading ? <div className="sec-desc">正在读取模型…</div> : null}
                {!modelsLoading && visibleModels.length === 0 ? (
                  <div className="panel-empty">
                    <Icon name="list-filter" size={20} />
                    <div>没有匹配的模型</div>
                    <div className="panel-empty-hint">调整关键字或能力筛选后再试。</div>
                  </div>
                ) : null}
                {!modelsLoading && visibleModels.length > 0 ? (
                  <table className="cap-table catalog-model-table">
                    <thead>
                      <tr>
                        <th />
                        <th>模型</th>
                        <th>上下文</th>
                        <th>价格 ($/M)</th>
                        <th>能力</th>
                      </tr>
                    </thead>
                    <tbody>
                      {visibleModels.map((item) => (
                        <tr key={item.id} onClick={() => toggleModel(item.id)}>
                          <td>
                            <input
                              type="checkbox"
                              checked={selected.has(item.id)}
                              aria-label={`选择 ${item.name}`}
                              onChange={() => toggleModel(item.id)}
                              onClick={(event) => event.stopPropagation()}
                            />
                          </td>
                          <td>
                            <div className="catalog-model-name">
                              {item.name}
                              {item.status ? <span className="badge neutral">{item.status}</span> : null}
                            </div>
                            <div className="mono sec-desc">{item.id}</div>
                          </td>
                          <td>{item.contextK}k</td>
                          <td>
                            {item.cost
                              ? `$${item.cost.inputPerMTok} / $${item.cost.outputPerMTok}`
                              : "—"}
                          </td>
                          <td className="catalog-caps">
                            {[
                              item.caps.tools ? "工具" : "",
                              item.caps.reasoning ? "推理" : "",
                              item.caps.vision ? "图片" : "",
                              item.caps.structured ? "结构化" : "",
                            ]
                              .filter(Boolean)
                              .join(" / ") || "—"}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                ) : null}
              </div>

              {error ? <div className="provider-editor-error">{error}</div> : null}

              <div className="provider-editor-actions">
                <span className="sec-desc">已选 {selected.size} 个模型</span>
                <span className="spacer" />
                <button type="button" className="btn ghost" onClick={onCancel}>
                  取消
                </button>
                <button
                  type="button"
                  className="btn primary"
                  disabled={importing || selected.size === 0 || Boolean(selectedProvider.added)}
                  onClick={() => void importSelected()}
                >
                  {importing ? <Icon name="loader" className="ico spin" /> : null}
                  导入提供商
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      {meta ? (
        <div className="sec-desc catalog-foot">
          目录：models.dev · {meta.generatedAt.slice(0, 10)} · {meta.providerCount} 个提供商 ·{" "}
          {meta.modelCount} 个模型 · {meta.license}
        </div>
      ) : null}
    </div>
  );
}
