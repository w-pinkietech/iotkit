package siteauth

import (
	"strings"
	"testing"
)

func TestHashAndVerifyPassword(t *testing.T) {
	const password = "現場で使う 長いパスワード"

	encoded, err := HashPassword(password)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(encoded, password) {
		t.Fatal("encoded password contains plaintext")
	}
	if !strings.HasPrefix(encoded, "$argon2id$v=19$m=65536,t=3,p=1$") {
		t.Fatalf("encoded password has unexpected parameters: %q", encoded)
	}

	ok, needsRehash, err := VerifyPassword(encoded, password)
	if err != nil {
		t.Fatal(err)
	}
	if !ok || needsRehash {
		t.Fatalf("VerifyPassword = (%v, %v), want (true, false)", ok, needsRehash)
	}

	ok, needsRehash, err = VerifyPassword(encoded, "別の長いパスワード")
	if err != nil {
		t.Fatal(err)
	}
	if ok || needsRehash {
		t.Fatalf("wrong password verification = (%v, %v), want (false, false)", ok, needsRehash)
	}
}

func TestVerifyPasswordRejectsMalformedEncoding(t *testing.T) {
	if _, _, err := VerifyPassword("$argon2id$broken", "some password"); err == nil {
		t.Fatal("VerifyPassword accepted malformed encoding")
	}
}

func TestValidatePasswordUsesLengthWithoutCompositionRules(t *testing.T) {
	for _, password := range []string{
		"abcdefghijkl",
		"これは十分に長い合言葉です",
		"factory floor password",
	} {
		if err := ValidatePassword(password); err != nil {
			t.Fatalf("ValidatePassword(%q) error = %v", password, err)
		}
	}
	if err := ValidatePassword("short"); err == nil {
		t.Fatal("ValidatePassword accepted fewer than 12 characters")
	}
	if err := ValidatePassword(strings.Repeat("a", 129)); err == nil {
		t.Fatal("ValidatePassword accepted more than 128 characters")
	}
}

func TestNormalizeLoginID(t *testing.T) {
	got, err := NormalizeLoginID("Operator.One")
	if err != nil {
		t.Fatal(err)
	}
	if got != "operator.one" {
		t.Fatalf("NormalizeLoginID = %q, want operator.one", got)
	}
	for _, invalid := range []string{"ab", "担当者", "operator one", "operator/one"} {
		if _, err := NormalizeLoginID(invalid); err == nil {
			t.Fatalf("NormalizeLoginID(%q) succeeded", invalid)
		}
	}
}
