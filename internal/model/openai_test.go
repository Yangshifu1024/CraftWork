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
