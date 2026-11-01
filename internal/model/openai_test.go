package model

import (
	"strings"
	"testing"
)

func TestOpenAIUserContentWithAttachments(t *testing.T) {
	client := NewOpenAICompat(Provider{ID: "openai", Name: "OpenAI", Kind: "openai"}, "http://example.com", "key")
	body, err := client.requestBody(Request{
		Model: "gpt-4o",
		Messages: []Message{{
			Role:    "user",
			Content: "看看附件",
			Attachments: []Attachment{
				{Name: "note.txt", Text: "hello"},
				{Name: "pic.png", MIME: "image/png", Data: "cGlj"},
				{Name: "skip.txt", Skipped: "太大"},
			},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := body["messages"].([]map[string]any)
	if len(messages) != 1 {
		t.Fatalf("消息数量不符: %+v", messages)
	}
	parts, ok := messages[0]["content"].([]map[string]any)
	if !ok {
		t.Fatalf("带图片的消息应使用内容块: %+v", messages[0]["content"])
	}
	if len(parts) != 2 {
		t.Fatalf("内容块数量不符: %+v", parts)
	}
	text, _ := parts[0]["text"].(string)
	if !strings.Contains(text, "hello") || !strings.Contains(text, "不可信") || !strings.Contains(text, "未读取") {
		t.Fatalf("文本内容块不符: %q", text)
	}
	image := parts[1]["image_url"].(map[string]any)
	if !strings.Contains(image["url"].(string), "data:image/png;base64,cGlj") {
		t.Fatalf("图片内容块不符: %+v", parts[1])
	}
}

func TestOpenAIUserContentWithoutImageReturnsText(t *testing.T) {
	client := NewOpenAICompat(Provider{ID: "openai", Name: "OpenAI", Kind: "openai"}, "http://example.com", "key")
	body, err := client.requestBody(Request{
		Model: "gpt-4o",
		Messages: []Message{{
			Role:        "user",
			Content:     "看看附件",
			Attachments: []Attachment{{Name: "note.txt", Text: "hello"}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := body["messages"].([]map[string]any)
	text, ok := messages[0]["content"].(string)
	if !ok || !strings.Contains(text, "hello") {
		t.Fatalf("纯文本附件应使用字符串内容: %+v", messages[0]["content"])
	}
}

// TestOpenAISystemBlocksJoined 验证系统分块的拼接规则：空白块跳过，
// 其余块用空行拼成第一条 system 消息；CacheControl 在该协议下没有输出。
func TestOpenAISystemBlocksJoined(t *testing.T) {
	client := NewOpenAICompat(Provider{ID: "openai", Name: "OpenAI", Kind: "openai"}, "http://example.com", "key")
	body, err := client.requestBody(Request{
		Model:  "gpt-4o",
		System: []SystemBlock{{Content: "你是助手。"}, {Content: "  "}, {Content: "行为规则", CacheControl: true}},
		Messages: []Message{{
			Role:    "user",
			Content: "列出文件",
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := body["messages"].([]map[string]any)
	if len(messages) != 2 {
		t.Fatalf("消息数量不符: %+v", messages)
	}
	if messages[0]["role"] != "system" {
		t.Fatalf("第一条应是 system 消息: %+v", messages[0])
	}
	text, _ := messages[0]["content"].(string)
	if text != "你是助手。\n\n行为规则" {
		t.Fatalf("system 拼接不符: %q", text)
	}
}

// TestOpenAIEmptySystemOmitted 验证全空白的系统提示词不会产生 system 消息。
func TestOpenAIEmptySystemOmitted(t *testing.T) {
	client := NewOpenAICompat(Provider{ID: "openai", Name: "OpenAI", Kind: "openai"}, "http://example.com", "key")
	body, err := client.requestBody(Request{
		Model:    "gpt-4o",
		System:   []SystemBlock{{Content: "  "}},
		Messages: []Message{{Role: "user", Content: "列出文件"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := body["messages"].([]map[string]any)
	if len(messages) != 1 || messages[0]["role"] != "user" {
		t.Fatalf("不应产生 system 消息: %+v", messages)
	}
}
