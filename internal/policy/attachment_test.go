package policy

import "testing"

func TestIsSensitiveAttachment(t *testing.T) {
	cases := []struct {
		name string
		want bool
	}{
		{".env", true},
		{".env.local", true},
		{"ID_RSA", true},
		{"id_ed25519", true},
		{"server.pem", true},
		{"client.KEY", true},
		{"cert.p12", true},
		{"note.txt", false},
		{"environment.md", false},
		{".envrc", false},
	}
	for _, tc := range cases {
		if got := IsSensitiveAttachment(tc.name); got != tc.want {
			t.Fatalf("IsSensitiveAttachment(%q) = %v, want %v", tc.name, got, tc.want)
		}
	}
}
