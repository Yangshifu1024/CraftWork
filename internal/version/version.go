// Package version 保存构建期注入的版本信息。
//
// 发布构建通过 -ldflags 覆盖这三个变量：
//
//	go build -ldflags "-X craftwork/internal/version.Version=1.2.3 \
//	  -X craftwork/internal/version.Commit=$(git rev-parse --short HEAD) \
//	  -X craftwork/internal/version.BuildTime=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
package version

// Info 是暴露给前端的构建信息，字段全部可 JSON 序列化。
type Info struct {
	Version   string `json:"version"`
	Commit    string `json:"commit"`
	BuildTime string `json:"buildTime"`
}

var (
	// Version 是产品版本号，默认值用于开发构建。
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
