//go:build production

package app

// contentSecurityPolicy 是生产构建的内容安全策略。
//
// 只允许加载本地内嵌资源；不加载任何远程脚本、样式、字体或连接。
// 外部链接必须通过 AppService.OpenExternal 交给系统浏览器。
func contentSecurityPolicy() string {
	return "default-src 'self'; " +
		"script-src 'self'; " +
		"style-src 'self' 'unsafe-inline'; " +
		"img-src 'self' data: blob:; " +
		"font-src 'self' data:; " +
		"connect-src 'self'; " +
		"object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
}
