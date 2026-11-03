// 提供商编辑器：新增或修改一个模型提供商（名称、协议、端点、是否需要
// 密钥、密钥）与它的模型列表，提交前做本地校验。
//
// 密钥不回显：编辑已有提供商时输入框始终为空，留空表示保留原密钥；
// 填写后与配置一起保存，明文只写入 Go 侧的 keys.json。模型行支持从
// models.dev 目录按 ID 补全上下文、能力与价格（后台查找）。

package overlays

import (
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// modelDraft 是表单里的模型草稿；ContextK / MaxTokens 用 float64 以便
// 直接绑定 NumberInput。
type modelDraft struct {
	id, name    string
	contextK    float64
	maxTokens   float64
	caps        model.Capabilities
	cost        *model.Cost
	status      string
	canonicalID string
	releaseDate string
	// lookup 是目录补全的进度提示；err 为真时按错误色展示。
	lookup     string
	lookupErr  bool
	lookupBusy bool
}

// editorState 是提供商编辑器的界面状态。
type editorState struct {
	open bool
	// provider 非空表示编辑已有提供商；nil 表示新增。
	provider    *service.ProviderView
	providerID  string
	name        string
	kind        string
	endpoint    string
	keyRequired bool
	apiKey      string
	showKey     bool
	models      []modelDraft
	err         string
	saving      bool
}

// kindOptions 是协议选项：只保留三种协议。
var kindOptions = []selectOption[string]{
	{"openai-compatible", "OpenAI 兼容"},
	{"openai", "OpenAI"},
	{"anthropic", "Anthropic"},
}

// capabilityOptions 是模型能力开关；流式固定开启，不在此列表。
var capabilityOptions = []struct {
	key   func(*model.Capabilities) *bool
	label string
}{
	{func(c *model.Capabilities) *bool { return &c.Tools }, "工具"},
	{func(c *model.Capabilities) *bool { return &c.Vision }, "图片"},
	{func(c *model.Capabilities) *bool { return &c.Reasoning }, "推理"},
	{func(c *model.Capabilities) *bool { return &c.Structured }, "结构化"},
}

// newEditorState 从提供商视图构造编辑器状态；prov 为 nil 表示新增，
// 模型列表以一行空白草稿起步。
func newEditorState(prov *service.ProviderView) editorState {
	st := editorState{open: true, provider: prov}
	if prov == nil {
		st.kind = "openai-compatible"
		st.keyRequired = true
		st.models = []modelDraft{blankModel()}
		return st
	}
	st.providerID = prov.ID
	st.name = prov.Name
	st.kind = prov.Kind
	st.endpoint = prov.Endpoint
	st.keyRequired = prov.KeyRequired
	for _, m := range prov.Models {
		draft := modelDraft{
			id:          m.ID,
			name:        m.Name,
			contextK:    float64(m.ContextK),
			maxTokens:   float64(m.MaxTokens),
			caps:        m.Caps,
			cost:        m.Cost,
			status:      m.Status,
			canonicalID: m.CanonicalID,
			releaseDate: m.ReleaseDate,
		}
		draft.caps.Stream = true
		st.models = append(st.models, draft)
	}
	if len(st.models) == 0 {
		st.models = []modelDraft{blankModel()}
	}
	return st
}

// blankModel 生成一行空白模型草稿（流式与工具默认开启）。
func blankModel() modelDraft {
	return modelDraft{
		contextK: 32,
		caps:     model.Capabilities{Stream: true, Tools: true},
	}
}

// drawProviderEditor 画提供商编辑器表单（替换模型分组的内容区）。
func (s *Store) drawProviderEditor(c *mygo.Context, st *settingsState) {
	ed := &st.editor
	if !ed.open {
		return
	}
	p := pal(c)

	title := "添加提供商"
	if ed.provider != nil {
		title = "编辑提供商"
	}
	mygo.Column(c).Fill().Gap(theme.Space6).Children(func() {
		// 头部：标题与关闭。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			icons.Icon(c, "cpu").Size(16, 16).TextColor(p.Text2)
			mygo.Text(c, title).FontSize(theme.Fonts.LG()).Bold()
			mygo.Spacer(c)
			if iconBtn(c, "x", "关闭提供商编辑器").Clicked() {
				st.editor = editorState{}
			}
		})
		if ed.provider != nil {
			mutedText(c, "提供商 ID："+ed.provider.ID)
		}

		// 基本信息。
		mygo.Field(c, "名称", func() {
			mygo.TextInput(c, &ed.name).Fill().Height(theme.HeightInput).
				Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
				Placeholder("例如 公司网关")
		}).Fill()
		mygo.Field(c, "协议", func() {
			enumSelect(c, &ed.kind, 220, kindOptions, nil)
		})
		mygo.Field(c, "端点", func() {
			mygo.TextInput(c, &ed.endpoint).Fill().Height(theme.HeightInput).
				Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
				Placeholder("https://api.example.com/v1")
		}).Fill()

		// 密钥设置：需要 API Key 勾选 + 掩码输入。
		mygo.Checkbox(c, &ed.keyRequired, "需要 API Key（本地服务如 Ollama 可以关闭）")
		if ed.keyRequired {
			mygo.Field(c, "API Key", func() {
				mygo.Row(c).Fill().Gap(theme.Space3).AlignItems(mygo.Center).Children(func() {
					input := mygo.TextInput(c, &ed.apiKey).Grow(1).Height(theme.HeightInput).
						Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
						Label("API Key").Placeholder(keyPlaceholder(ed)).
						AutoFocus()
					if !ed.showKey {
						input.Password()
					}
					eye := "eye"
					if ed.showKey {
						eye = "eye-off"
					}
					iconBtn(c, eye, "显示或隐藏密钥")
				})
			})
			if ed.provider != nil && ed.provider.KeySource != "" {
				mutedText(c, "已配置 "+ed.provider.KeyMasked+"；留空则保持不变")
			}
		}

		// 模型列表。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			mygo.Text(c, "模型").Bold()
			mutedText(c, "流式固定开启。").Grow(1)
			if smallGhostBtn(c, "添加模型").Clicked() {
				ed.models = append(ed.models, blankModel())
			}
		})
		for i := range ed.models {
			s.modelRow(c, ed, i)
		}

		if ed.err != "" {
			mygo.Text(c, ed.err).TextColor(p.Danger)
		}

		// 底部操作。
		mygo.Row(c).Fill().Justify(mygo.End).Gap(theme.Space4).Children(func() {
			if mygo.Button(c, "取消").Clicked() {
				st.editor = editorState{}
			}
			save := primaryBtn(c, "保存提供商").Disabled(ed.saving)
			if save.Clicked() {
				s.saveProvider(c, st)
			}
		})
	})
}

// keyPlaceholder 组装密钥输入框的占位提示。
func keyPlaceholder(ed *editorState) string {
	if ed.provider != nil && ed.provider.KeySource != "" {
		mask := ed.provider.KeyMasked
		if mask == "" {
			mask = "密钥"
		}
		return "已配置 " + mask + "；留空则保持不变"
	}
	return "粘贴 API Key"
}

// modelRow 画一行模型表单：ID / 名称 / 上下文 / 最大输出 / 能力勾选，
// 以及「目录补全」按钮与进度提示。
func (s *Store) modelRow(c *mygo.Context, ed *editorState, index int) {
	p := pal(c)
	m := &ed.models[index]
	mygo.Column(c).Fill().Gap(theme.Space4).Padding(theme.Space5).
		Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.Border).Children(func() {
		head := mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			mygo.Text(c, fmt.Sprintf("模型 %d", index+1)).TextColor(p.Text2)
			mygo.Spacer(c)
			rm := iconBtn(c, "trash", fmt.Sprintf("移除模型 %d", index+1)).Disabled(len(ed.models) <= 1)
			_ = rm
			if len(ed.models) > 1 && rm.Clicked() {
				ed.models = append(ed.models[:index], ed.models[index+1:]...)
			}
		})
		_ = head

		mygo.Row(c).Fill().Gap(theme.Space5).Children(func() {
			mygo.Field(c, "模型 ID", func() {
				mygo.TextInput(c, &m.id).Width(170).Height(theme.HeightInput).
					Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
					Label(fmt.Sprintf("模型 %d ID", index+1)).
					Placeholder("例如 gpt-4.1")
			})
			mygo.Field(c, "显示名称", func() {
				mygo.TextInput(c, &m.name).Width(150).Height(theme.HeightInput).
					Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
					Placeholder("例如 GPT-4.1")
			})
			mygo.Field(c, "上下文（千 token）", func() {
				mygo.NumberInput(c, &m.contextK, 1, 100000, 1).Width(120)
			})
			mygo.Field(c, "最大输出（token）", func() {
				mygo.NumberInput(c, &m.maxTokens, 0, 4000000, 1).Width(120)
			})
			fill := smallGhostBtn(c, "目录补全").Disabled(m.lookupBusy)
			if fill.Clicked() {
				s.fillFromCatalog(ed, index)
			}
		})

		mygo.Row(c).Gap(theme.Space6).Children(func() {
			m.caps.Stream = true
			for _, opt := range capabilityOptions {
				mygo.Checkbox(c, opt.key(&m.caps), opt.label)
			}
		})
		if m.lookup != "" {
			color := p.OK
			if m.lookupErr {
				color = p.Danger
			}
			mygo.Text(c, m.lookup).FontSize(theme.Fonts.SM()).TextColor(color)
		}
	})
}

// fillFromCatalog 按模型 ID 从目录补全上下文、能力与价格（后台查找）。
func (s *Store) fillFromCatalog(ed *editorState, index int) {
	m := &ed.models[index]
	modelID := strings.TrimSpace(m.id)
	if modelID == "" {
		return
	}
	m.lookupBusy = true
	m.lookup = "正在查找目录…"
	m.lookupErr = false
	s.runAsync(func() func() {
		found, err := s.be.Providers.LookupCatalogModel(ed.providerID, modelID)
		return func() {
			m := &ed.models[index]
			m.lookupBusy = false
			switch {
			case err != nil:
				m.lookup, m.lookupErr = err.Error(), true
			case found == nil:
				m.lookup, m.lookupErr = "目录中没有这个模型", true
			default:
				m.name = strings.TrimSpace(m.name)
				if m.name == "" {
					m.name = found.Name
				}
				m.contextK = float64(found.ContextK)
				m.maxTokens = float64(found.MaxTokens)
				m.caps = found.Caps
				m.caps.Stream = true
				m.cost = found.Cost
				m.status = found.Status
				m.canonicalID = found.CanonicalID
				m.releaseDate = found.ReleaseDate
				m.lookup = "已从 " + found.ProviderName + " 目录补全"
				m.lookupErr = false
			}
		}
	})
}

// saveProvider 校验并保存表单；填了密钥时紧接着写入 keys.json。
func (s *Store) saveProvider(c *mygo.Context, st *settingsState) {
	ed := &st.editor
	name := strings.TrimSpace(ed.name)
	endpoint := strings.TrimSpace(ed.endpoint)
	switch {
	case name == "":
		ed.err = "请填写提供商名称"
		return
	case endpoint == "":
		ed.err = "请填写端点地址"
		return
	}
	inputs := make([]service.ModelInput, 0, len(ed.models))
	for i := range ed.models {
		m := &ed.models[i]
		id := strings.TrimSpace(m.id)
		if id == "" {
			ed.err = "模型 ID 不能为空"
			return
		}
		if m.contextK <= 0 {
			ed.err = "上下文窗口必须大于 0"
			return
		}
		inputs = append(inputs, service.ModelInput{
			ID:          id,
			Name:        strings.TrimSpace(m.name),
			ContextK:    int(m.contextK),
			MaxTokens:   int(m.maxTokens),
			Caps:        m.caps,
			Cost:        m.cost,
			Status:      m.status,
			CanonicalID: m.canonicalID,
			ReleaseDate: m.releaseDate,
		})
	}
	if len(inputs) == 0 {
		ed.err = "至少添加一个模型"
		return
	}

	trimmedKey := strings.TrimSpace(ed.apiKey)
	writeKey := ed.keyRequired && trimmedKey != ""

	ed.saving = true
	ed.err = ""
	s.runAsync(func() func() {
		input := service.ProviderInput{
			ID:          ed.providerID,
			Name:        name,
			Kind:        ed.kind,
			Endpoint:    endpoint,
			KeyRequired: ed.keyRequired,
			Models:      inputs,
		}
		saved, err := s.be.Providers.SaveProvider(input)
		if err == nil && writeKey {
			err = s.be.Providers.SetKey(saved.ID, trimmedKey)
		}
		return func() {
			ed := &st.editor
			ed.saving = false
			if err != nil {
				ed.err = err.Error()
				return
			}
			// 重新拉取列表，拿到最新的密钥状态再收起编辑器。
			st.providers = s.providerList()
			savedView := s.findProvider(st, saved.ID)
			st.editor = editorState{}
			label := "提供商已添加"
			if ed.provider != nil {
				label = "提供商已更新"
			}
			desc := fmt.Sprintf("%s · %d 个模型", saved.Name, len(saved.Models))
			if writeKey {
				desc += " · 密钥已保存"
			} else if savedView != nil && savedView.KeyRequired && savedView.KeySource == "" {
				Notify("还需配置 API Key", "在提供商卡片中展开并设置密钥后，模型才会出现在模型列表。", ToneWarn)
			}
			Notify(label, desc, ToneOK)
			// 还没有默认模型时自动选中新提供商的第一个模型（仅补一次）。
			if st.draft.DefaultModel == "" && len(saved.Models) > 0 {
				s.save(c, func(x *service.Settings) { x.DefaultModel = saved.Models[0].Ref })
			}
		}
	})
}

// findProvider 从状态里的列表按 ID 找提供商视图。
func (s *Store) findProvider(st *settingsState, id string) *service.ProviderView {
	for i := range st.providers {
		if st.providers[i].ID == id {
			return &st.providers[i]
		}
	}
	return nil
}
