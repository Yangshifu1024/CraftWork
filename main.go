// CraftWork 桌面入口。
//
// 该文件只做两件事：把前端构建产物嵌入二进制，然后把应用装配交给
// internal/app。真正的窗口、服务、事件与生命周期都在 internal/app 中。
package main

import (
	"embed"
	"log"

	"craftwork/internal/app"
	"craftwork/internal/version"
)

// frontend/dist 由 `wails3 build` 或 `npm run build` 生成。
// 生产构建时这些文件会被完整嵌入二进制，由 Wails3 的资源服务器提供。
//
//go:embed all:frontend/dist
var assets embed.FS

func main() {
	if err := app.Run(assets, version.Current()); err != nil {
		log.Fatal(err)
	}
}
