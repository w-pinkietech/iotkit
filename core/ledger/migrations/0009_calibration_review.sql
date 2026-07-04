-- 計画4 T2(D3「較正要再確認状態の列は初日から」の回収。D5決定2/replace-hardware動線)
ALTER TABLE series ADD COLUMN calibration_review INTEGER NOT NULL DEFAULT 0;
