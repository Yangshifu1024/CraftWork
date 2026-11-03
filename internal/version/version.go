// Package version 保存构建期注入的版本信息。
//
// 版本对齐策略（10 号票）：mygo.json 的 version 是发布产物（.app /
// DMG / 更新通道）的版本事实源；本包 Version 的默认值必须与之保持一致
// （发版时两处同改）。差异只在开发构建出现：`go tool mygo build` 不注入
// ldflags，包内默认值即生效；需要带精确版本的开发构建用 -ldflags 覆盖：
//
//	go build -ldflags "-X craftwork/internal/version.Version=1.2.3 \
//	  -X craftwork/internal/version.Commit=$(git rev-parse --short HEAD) \
//	  -X craftwork/internal/version.BuildTime=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
//
// mygo.json 是 JSON，写不了注释，对齐规则以本文件为准。
package version

// Info 是暴露给设置中心「关于」分组的构建信息，字段全部可 JSON 序列化。
type Info struct {
	Version   string `json:"version"`
	Commit    string `json:"commit"`
	BuildTime string `json:"buildTime"`
}

var (
	// Version 是产品版本号，默认值与 mygo.json 的 version 对齐，
	// 发布前若改版本需两处同改。
	Version = "0.1.0"
	// Commit 是构建对应的 Git 提交，开发构建为 dev。
	Commit = "dev"
	// BuildTime 是 UTC 构建时间，开发构建为空。
	BuildTime = ""
)

// Current 返回当前构建信息。
func Current() Info {
	return Info{Version: Version, Commit: Commit, BuildTime: BuildTime}
}
