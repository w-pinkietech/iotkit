package sitehttp

import "testing"

func TestFormatByteCountUsesBinaryUnits(t *testing.T) {
	tests := map[uint64]string{
		0:                             "0 B",
		1024:                          "1.0 KiB",
		1024 * 1024:                   "1.0 MiB",
		1024 * 1024 * 1024:            "1.0 GiB",
		5 * 1024 * 1024 * 1024:        "5.0 GiB",
		2 * 1024 * 1024 * 1024 * 1024: "2.0 TiB",
	}
	for input, expected := range tests {
		if got := formatByteCount(input); got != expected {
			t.Errorf("formatByteCount(%d)=%q, want %q", input, got, expected)
		}
	}
}
