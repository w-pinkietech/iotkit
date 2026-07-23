package edgehttp

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

func TestConsoleOnboardingPointsAtTheFirstIncompleteStep(t *testing.T) {
	tests := []struct {
		name      string
		facts     consoleOnboardingFacts
		wantTitle string
		wantHref  string
		wantDone  int
	}{
		{
			name:      "collection node",
			facts:     consoleOnboardingFacts{},
			wantTitle: "収集ノードを登録",
			wantHref:  "/equipment",
		},
		{
			name: "device profile",
			facts: consoleOnboardingFacts{
				ActiveEdgeNodes: 1,
				DeviceCount:     1,
				PendingDevices:  1,
			},
			wantTitle: "デバイス名と設置場所を設定",
			wantHref:  "/equipment",
			wantDone:  1,
		},
		{
			name: "sensor profile",
			facts: consoleOnboardingFacts{
				ActiveEdgeNodes:     1,
				DeviceCount:         1,
				SignalCount:         1,
				UnconfiguredSignals: 1,
			},
			wantTitle: "センサーの種類と単位を確認",
			wantHref:  "/equipment",
			wantDone:  2,
		},
		{
			name: "meaning",
			facts: consoleOnboardingFacts{
				ActiveEdgeNodes: 1,
				DeviceCount:     1,
				SignalCount:     1,
			},
			wantTitle: "センサーの値の使い方を設定",
			wantHref:  "/sensors",
			wantDone:  3,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			view := newConsoleOnboardingView(test.facts)
			if !view.Show || view.CompleteCount != test.wantDone ||
				view.NextTitle != test.wantTitle || view.NextHref != test.wantHref {
				t.Fatalf("view = %#v", view)
			}
			if len(view.Steps) != 4 {
				t.Fatalf("steps = %d, want 4", len(view.Steps))
			}
			current := 0
			for _, step := range view.Steps {
				if step.Current {
					current++
				}
			}
			if current != 1 {
				t.Fatalf("current steps = %d, want 1: %#v", current, view.Steps)
			}
		})
	}
}

func TestConsoleOnboardingHidesAfterCoreSetupIsComplete(t *testing.T) {
	view := newConsoleOnboardingView(consoleOnboardingFacts{
		ActiveEdgeNodes: 1,
		DeviceCount:     1,
		SignalCount:     1,
		SemanticRules:   1,
	})

	if view.Show || view.CompleteCount != 4 || view.NextTitle != "" || view.NextHref != "" {
		t.Fatalf("view = %#v", view)
	}
}
