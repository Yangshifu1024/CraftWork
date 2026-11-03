// CraftWork 桌面入口。
//
// 该文件只做一件事：把应用装配交给 internal/app。真正的窗口、服务、
// 事件与生命周期都在 internal/app 中。
package main

import (
	"log"

	"github.com/yangshifu1024/craftwork/internal/app"
	"github.com/yangshifu1024/craftwork/internal/version"
)

func main() {
	if err := app.Run(version.Current()); err != nil {
		log.Fatal(err)
	}
}
