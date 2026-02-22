package handlers

import "strings"

type scoutingPointConfig struct {
	DefenseRating    map[string]int
	Traversal        map[string]int
	ShootingSpeed    map[string]int
	Capacity         map[string]int
	ScoringStrategy  map[string]int
	HangLevel        map[string]int
	AutoHang         map[string]int
	HangPosition     map[string]int
	StartingPosition map[string]int
}

var defaultScoutingPointConfig = scoutingPointConfig{
	DefenseRating: map[string]int{
		"high": 5,
		"mid":  3,
		"low":  1,
	},
	Traversal: map[string]int{
		"trench": 3,
		"bump":   2,
	},
	ShootingSpeed: map[string]int{
		"fast":   4,
		"medium": 2,
		"slow":   1,
	},
	Capacity: map[string]int{
		"high":   4,
		"medium": 2,
		"low":    1,
	},
	ScoringStrategy: map[string]int{
		"scoring":   4,
		"defending": 3,
		"passing":   2,
	},
	HangLevel: map[string]int{
		"l1": 2,
		"l2": 4,
		"l3": 6,
	},
	AutoHang: map[string]int{
		"yes": 3,
		"no":  0,
	},
	HangPosition: map[string]int{
		"left":   1,
		"center": 2,
		"right":  1,
	},
	StartingPosition: map[string]int{
		"left":   1,
		"center": 2,
		"right":  1,
	},
}

func calculateScoutingPoints(row scoutingMetricRow) int {
	cfg := defaultScoutingPointConfig
	total := 0

	total += cfg.DefenseRating[normalizeOption(row.DefenseRating)]
	total += cfg.Traversal[normalizeOption(row.Traversal)]
	total += cfg.ShootingSpeed[normalizeOption(row.ShootingSpeed)]
	total += cfg.Capacity[normalizeOption(row.Capacity)]
	total += cfg.ScoringStrategy[normalizeOption(row.ScoringStrategy)]
	total += cfg.HangLevel[normalizeOption(row.HangLevel)]
	total += cfg.AutoHang[normalizeOption(row.AutoHang)]
	total += cfg.HangPosition[normalizeOption(row.HangPosition)]
	total += cfg.StartingPosition[normalizeOption(row.StartingPosition)]

	return total
}

func normalizeOption(value string) string {
	return strings.ToLower(strings.TrimSpace(value))
}
