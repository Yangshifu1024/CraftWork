//go:build !production

package app

// contentSecurityPolicy 是开发构建的内容安全策略。
//
// 开发模式下页面由 Vite 提供，HMR 需要内联脚本与本地 WebSocket，
// 因此比生产策略宽松；生产策略见 csp_prod.go。
func contentSecurityPolicy() string {
	return "default-src 'self'; " +
		"script-src 'self' 'unsafe-inline' 'unsafe-eval'; " +
		"style-src 'self' 'unsafe-inline'; " +
		"img-src 'self' data: blob:; " +
		"font-src 'self' data:; " +
		"connect-src 'self' ws://localhost:* http://localhost:* ws://127.0.0.1:* http://127.0.0.1:*; " +
		"object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
}
