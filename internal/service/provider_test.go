package service

import (
	"testing"

	"craftwork/internal/model"
)

func TestConnectionRequestUsesProtocolHeaders(t *testing.T) {
	anthropic := model.Provider{
		ID: "anthropic", Name: "Anthropic", Kind: "anthropic",
		Endpoint: "https://api.anthropic.com",
	}
	request, err := connectionRequest(anthropic, "anthropic-key")
	if err != nil {
		t.Fatal(err)
	}
	if got, want := request.URL.String(), "https://api.anthropic.com/v1/models?limit=1"; got != want {
		t.Fatalf("Anthropic 地址不符: got %s want %s", got, want)
	}
	if got := request.Header.Get("x-api-key"); got != "anthropic-key" {
		t.Fatalf("Anthropic 密钥头不符: %q", got)
	}
	if got := request.Header.Get("anthropic-version"); got != "2023-06-01" {
		t.Fatalf("Anthropic 版本头不符: %q", got)
	}
	if got := request.URL.Query().Get("key"); got != "" {
		t.Fatalf("密钥不应出现在 URL 中: %q", got)
	}

	openai := model.Provider{
		ID: "openai", Name: "OpenAI", Kind: "openai",
		Endpoint: "https://api.openai.com/v1",
	}
	request, err = connectionRequest(openai, "openai-key")
	if err != nil {
		t.Fatal(err)
	}
	if got, want := request.URL.String(), "https://api.openai.com/v1/models"; got != want {
		t.Fatalf("OpenAI 地址不符: got %s want %s", got, want)
	}
	if got := request.Header.Get("Authorization"); got != "Bearer openai-key" {
		t.Fatalf("OpenAI 鉴权头不符: %q", got)
	}
}

func TestJoinServiceURLAvoidsDuplicateVersion(t *testing.T) {
	cases := []struct{ base, path, want string }{
		{"https://api.anthropic.com", "/v1/models", "https://api.anthropic.com/v1/models"},
		{"https://api.anthropic.com/", "/v1/models", "https://api.anthropic.com/v1/models"},
		{"https://api.anthropic.com/v1", "/v1/models", "https://api.anthropic.com/v1/models"},
	}
	for _, tc := range cases {
		if got := joinServiceURL(tc.base, tc.path); got != tc.want {
			t.Fatalf("拼接不符: got %s want %s", got, tc.want)
		}
	}
}
