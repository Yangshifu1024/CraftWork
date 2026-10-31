package notify

import "testing"

func TestAppleScriptStringEscapes(t *testing.T) {
	got := appleScriptString(`路径 "C:\tmp"`)
	want := `"路径 \"C:\\tmp\""`
	if got != want {
		t.Fatalf("转义不符: got %s want %s", got, want)
	}
}
