// modelinfo.go 是 Composer 对模型目录的只读查询：视觉能力、上下文窗口
// 与显示名。目录来自 internal/model 的构建期快照，查询只在主线程发生；
// 小缓存避免每帧线性扫描目录表。

package chat

import (
	"github.com/yangshifu1024/craftwork/internal/model"
)

// modelLite 是 Composer 用到的模型目录字段子集。
type modelLite struct {
	Name     string
	Vision   bool
	ContextK int
}

// modelCache 按「提供商/模型」引用缓存目录查询结果；找不到的引用缓存
// ok=false，避免逐帧重复扫描。只在主线程读写，无需加锁。
var modelCache = map[string]struct {
	info modelLite
	ok   bool
}{}

// modelByRef 按引用查模型目录（FindModelByRef 的缓存包装；旧数据的
// 原始 ID 回退交给 model.Resolve——只在这里的兜底路径出现）。
func modelByRef(ref string) (model.Provider, model.ModelInfo, bool) {
	if entry, cached := modelCache[ref]; cached {
		if !entry.ok {
			return model.Provider{}, model.ModelInfo{}, false
		}
		// 命中缓存时不需要 Provider，返回零值 Provider 与合成信息。
		return model.Provider{}, model.ModelInfo{Name: entry.info.Name, ContextK: entry.info.ContextK, Caps: model.Capabilities{Vision: entry.info.Vision}}, true
	}
	provider, info, found := model.Resolve(ref)
	modelCache[ref] = struct {
		info modelLite
		ok   bool
	}{info: modelLite{Name: info.Name, Vision: info.Caps.Vision, ContextK: info.ContextK}, ok: found}
	return provider, info, found
}
