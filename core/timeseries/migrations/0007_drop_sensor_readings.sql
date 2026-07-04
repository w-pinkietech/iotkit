-- 計画4 T1: 旧v2テーブル撤去。本番呼び出しゼロを確認済み(2026-07-03調査)。
-- 既存DB(開発機)にはテーブルが存在し、新規DBではv2マイグレーション自体を撤去するため
-- IF EXISTSで両対応する。
DROP TABLE IF EXISTS sensor_readings;
