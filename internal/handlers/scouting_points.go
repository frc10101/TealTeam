package handlers

import (
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/gin-gonic/gin"
)

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

type scoutingPointOption struct {
	Key    string
	Points int
}

type scoutingPointSection struct {
	Key     string
	Label   string
	Options []scoutingPointOption
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
		"none": 0,
		"l1":   2,
		"l2":   4,
		"l3":   6,
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

var scoutingPointSectionOrder = []scoutingPointSection{
	{Key: "defense_rating", Label: "Defense Rating", Options: []scoutingPointOption{{Key: "high"}, {Key: "mid"}, {Key: "low"}}},
	{Key: "traversal", Label: "Traversal", Options: []scoutingPointOption{{Key: "trench"}, {Key: "bump"}}},
	{Key: "shooting_speed", Label: "Shooting Speed", Options: []scoutingPointOption{{Key: "fast"}, {Key: "medium"}, {Key: "slow"}}},
	{Key: "capacity", Label: "Capacity", Options: []scoutingPointOption{{Key: "high"}, {Key: "medium"}, {Key: "low"}}},
	{Key: "scoring_strategy", Label: "Scoring Strategy", Options: []scoutingPointOption{{Key: "scoring"}, {Key: "defending"}, {Key: "passing"}}},
	{Key: "hang_level", Label: "Hang Level", Options: []scoutingPointOption{{Key: "none"}, {Key: "l1"}, {Key: "l2"}, {Key: "l3"}}},
	{Key: "auto_hang", Label: "Auto Hang", Options: []scoutingPointOption{{Key: "yes"}, {Key: "no"}}},
	{Key: "hang_position", Label: "Hang Position", Options: []scoutingPointOption{{Key: "left"}, {Key: "center"}, {Key: "right"}}},
	{Key: "starting_position", Label: "Starting Position", Options: []scoutingPointOption{{Key: "left"}, {Key: "center"}, {Key: "right"}}},
}

func calculateScoutingPoints(row scoutingMetricRow) int {
	return calculateScoutingPointsWithConfig(row, defaultScoutingPointConfig)
}

func calculateScoutingPointsWithConfig(row scoutingMetricRow, cfg scoutingPointConfig) int {
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

func cloneDefaultScoutingPointConfig() scoutingPointConfig {
	cloneMap := func(src map[string]int) map[string]int {
		dst := make(map[string]int, len(src))
		for key, value := range src {
			dst[key] = value
		}
		return dst
	}

	return scoutingPointConfig{
		DefenseRating:    cloneMap(defaultScoutingPointConfig.DefenseRating),
		Traversal:        cloneMap(defaultScoutingPointConfig.Traversal),
		ShootingSpeed:    cloneMap(defaultScoutingPointConfig.ShootingSpeed),
		Capacity:         cloneMap(defaultScoutingPointConfig.Capacity),
		ScoringStrategy:  cloneMap(defaultScoutingPointConfig.ScoringStrategy),
		HangLevel:        cloneMap(defaultScoutingPointConfig.HangLevel),
		AutoHang:         cloneMap(defaultScoutingPointConfig.AutoHang),
		HangPosition:     cloneMap(defaultScoutingPointConfig.HangPosition),
		StartingPosition: cloneMap(defaultScoutingPointConfig.StartingPosition),
	}
}

func (h *Handler) loadEffectiveScoutingPointConfig(c *gin.Context) scoutingPointConfig {
	config := cloneDefaultScoutingPointConfig()
	if !h.hasDB() {
		return config
	}

	type dbRow struct {
		MetricKey string
		OptionKey string
		Points    int
	}

	var rows []dbRow
	err := h.db.WithContext(c.Request.Context()).
		Table("scouting_point_weights").
		Select("metric_key, option_key, points").
		Scan(&rows).Error
	if err != nil {
		h.log.Warn("failed to load scouting point config; using defaults", "error", err)
		return config
	}

	for _, row := range rows {
		setConfigValue(&config, row.MetricKey, row.OptionKey, row.Points)
	}

	return config
}

func (h *Handler) loadScoutingPointSections(c *gin.Context) []scoutingPointSection {
	cfg := h.loadEffectiveScoutingPointConfig(c)
	sections := make([]scoutingPointSection, 0, len(scoutingPointSectionOrder))
	for _, section := range scoutingPointSectionOrder {
		options := make([]scoutingPointOption, 0, len(section.Options))
		for _, option := range section.Options {
			points, ok := getConfigValue(cfg, section.Key, option.Key)
			if !ok {
				continue
			}
			options = append(options, scoutingPointOption{Key: option.Key, Points: points})
		}
		if len(options) == 0 {
			continue
		}
		sections = append(sections, scoutingPointSection{Key: section.Key, Label: section.Label, Options: options})
	}
	return sections
}

func (h *Handler) persistScoutingPointSections(c *gin.Context, sections []scoutingPointSection) error {
	if !h.hasDB() {
		return nil
	}

	tx := h.db.WithContext(c.Request.Context()).Begin()
	if tx.Error != nil {
		return tx.Error
	}

	for _, section := range sections {
		for _, option := range section.Options {
			if err := tx.Exec(`
				INSERT INTO scouting_point_weights (metric_key, option_key, points)
				VALUES (?, ?, ?)
				ON CONFLICT (metric_key, option_key)
				DO UPDATE SET points = EXCLUDED.points, updated_at = CURRENT_TIMESTAMP
			`, section.Key, option.Key, option.Points).Error; err != nil {
				_ = tx.Rollback()
				return err
			}
		}
	}

	return tx.Commit().Error
}

func parsePointSectionsFromForm(c *gin.Context, base []scoutingPointSection) ([]scoutingPointSection, error) {
	parsed := make([]scoutingPointSection, 0, len(base))
	for _, section := range base {
		parsedOptions := make([]scoutingPointOption, 0, len(section.Options))
		for _, option := range section.Options {
			fieldName := buildWeightFieldName(section.Key, option.Key)
			value := strings.TrimSpace(c.PostForm(fieldName))
			parsedValue := option.Points
			if value == "" {
				parsedValue = option.Points
			} else {
				intValue, err := strconv.Atoi(value)
				if err != nil {
					return nil, fmt.Errorf("invalid weight for %s: %w", fieldName, err)
				}
				if intValue < -100 || intValue > 100 {
					return nil, fmt.Errorf("weight out of range for %s", fieldName)
				}
				parsedValue = intValue
			}
			parsedOptions = append(parsedOptions, scoutingPointOption{Key: option.Key, Points: parsedValue})
		}
		parsed = append(parsed, scoutingPointSection{Key: section.Key, Label: section.Label, Options: parsedOptions})
	}
	return parsed, nil
}

func buildWeightFieldName(metricKey, optionKey string) string {
	return "weight_" + metricKey + "__" + optionKey
}

func getConfigValue(cfg scoutingPointConfig, metricKey, optionKey string) (int, bool) {
	metricKey = normalizeOption(metricKey)
	optionKey = normalizeOption(optionKey)

	switch metricKey {
	case "defense_rating":
		value, ok := cfg.DefenseRating[optionKey]
		return value, ok
	case "traversal":
		value, ok := cfg.Traversal[optionKey]
		return value, ok
	case "shooting_speed":
		value, ok := cfg.ShootingSpeed[optionKey]
		return value, ok
	case "capacity":
		value, ok := cfg.Capacity[optionKey]
		return value, ok
	case "scoring_strategy":
		value, ok := cfg.ScoringStrategy[optionKey]
		return value, ok
	case "hang_level":
		value, ok := cfg.HangLevel[optionKey]
		return value, ok
	case "auto_hang":
		value, ok := cfg.AutoHang[optionKey]
		return value, ok
	case "hang_position":
		value, ok := cfg.HangPosition[optionKey]
		return value, ok
	case "starting_position":
		value, ok := cfg.StartingPosition[optionKey]
		return value, ok
	default:
		return 0, false
	}
}

func setConfigValue(cfg *scoutingPointConfig, metricKey, optionKey string, points int) {
	metricKey = normalizeOption(metricKey)
	optionKey = normalizeOption(optionKey)

	switch metricKey {
	case "defense_rating":
		cfg.DefenseRating[optionKey] = points
	case "traversal":
		cfg.Traversal[optionKey] = points
	case "shooting_speed":
		cfg.ShootingSpeed[optionKey] = points
	case "capacity":
		cfg.Capacity[optionKey] = points
	case "scoring_strategy":
		cfg.ScoringStrategy[optionKey] = points
	case "hang_level":
		cfg.HangLevel[optionKey] = points
	case "auto_hang":
		cfg.AutoHang[optionKey] = points
	case "hang_position":
		cfg.HangPosition[optionKey] = points
	case "starting_position":
		cfg.StartingPosition[optionKey] = points
	}
}

func optionLabel(optionKey string) string {
	parts := strings.Split(optionKey, "_")
	for i, part := range parts {
		if part == "" {
			continue
		}
		parts[i] = strings.ToUpper(part[:1]) + part[1:]
	}
	return strings.Join(parts, " ")
}

func sortSectionsByLabel(sections []scoutingPointSection) {
	sort.SliceStable(sections, func(i, j int) bool {
		return sections[i].Label < sections[j].Label
	})
}

func normalizeOption(value string) string {
	return strings.ToLower(strings.TrimSpace(value))
}
