package models

import "time"

// Item is a generic example model
// TODO: Replace or extend with your domain-specific models
type Item struct {
	ID          int       `json:"id"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

// User represents an authenticated user
type User struct {
	ID           int        `json:"id" gorm:"column:id;primaryKey"`
	Email        string     `json:"email" gorm:"column:email"`
	Name         string     `json:"name" gorm:"column:name"`
	PasswordHash string     `json:"-" gorm:"column:password_hash"` // Never send password hash in JSON
	TeamNumber   *int       `json:"team_number,omitempty" gorm:"column:team_number"`
	Role         string     `json:"role" gorm:"column:role"`
	IsAdmin      bool       `json:"is_admin" gorm:"column:is_admin"`
	IsLeadScout  bool       `json:"is_lead_scout" gorm:"column:is_lead_scout"`
	IsCoach      bool       `json:"is_coach" gorm:"column:is_coach"`
	LastLogin    *time.Time `json:"last_login,omitempty" gorm:"column:last_login"`
	CreatedAt    time.Time  `json:"created_at" gorm:"column:created_at"`
	UpdatedAt    time.Time  `json:"updated_at" gorm:"column:updated_at"`
}

// Session represents a user session
type Session struct {
	SessionID       string    `gorm:"column:session_id;primaryKey"`
	UserID          int       `gorm:"column:user_id"`
	SelectedEventID *int      `gorm:"column:selected_event_id"`
	ExpiresAt       time.Time `gorm:"column:expires_at"`
	CreatedAt       time.Time `gorm:"column:created_at"`
}

// Team represents a FRC team
type Team struct {
	ID         int       `json:"id" gorm:"column:id;primaryKey"`
	TeamNumber int       `json:"team_number" gorm:"column:team_number"`
	Name       string    `json:"name" gorm:"column:name"`
	School     *string   `json:"school,omitempty" gorm:"column:school"`
	City       *string   `json:"city,omitempty" gorm:"column:city"`
	State      *string   `json:"state,omitempty" gorm:"column:state"`
	TBAKey     *string   `json:"tba_key,omitempty" gorm:"column:tba_key"`
	Nickname   *string   `json:"nickname,omitempty" gorm:"column:nickname"`
	SchoolName *string   `json:"school_name,omitempty" gorm:"column:school_name"`
	Country    *string   `json:"country,omitempty" gorm:"column:country"`
	RookieYear *int      `json:"rookie_year,omitempty" gorm:"column:rookie_year"`
	Motto      *string   `json:"motto,omitempty" gorm:"column:motto"`
	Website    *string   `json:"website,omitempty" gorm:"column:website"`
	CreatedAt  time.Time `json:"created_at" gorm:"column:created_at"`
	UpdatedAt  time.Time `json:"updated_at" gorm:"column:updated_at"`
}

// Event represents a FRC competition event
type Event struct {
	ID          int        `json:"id" gorm:"column:id;primaryKey"`
	Name        string     `json:"name" gorm:"column:name"`
	Location    *string    `json:"location,omitempty" gorm:"column:location"`
	StartDate   *time.Time `json:"start_date,omitempty" gorm:"column:start_date"`
	EndDate     *time.Time `json:"end_date,omitempty" gorm:"column:end_date"`
	TBAKey      *string    `json:"tba_key,omitempty" gorm:"column:tba_key"`
	EventType   *string    `json:"event_type,omitempty" gorm:"column:event_type"`
	DistrictKey *string    `json:"district_key,omitempty" gorm:"column:district_key"`
	Week        *int       `json:"week,omitempty" gorm:"column:week"`
	CreatedAt   time.Time  `json:"created_at" gorm:"column:created_at"`
	UpdatedAt   time.Time  `json:"updated_at" gorm:"column:updated_at"`
}

// ScoutingData represents scouting data for a team at a match
type ScoutingData struct {
	ID               int        `json:"id" gorm:"column:id;primaryKey"`
	EventID          int        `json:"event_id" gorm:"column:event_id"`
	TeamID           int        `json:"team_id" gorm:"column:team_id"`
	AllianceColor    string     `json:"alliance_color" gorm:"column:alliance_color"`
	AutoScore        int        `json:"auto_score" gorm:"column:auto_score"`
	TeleopScore      int        `json:"teleop_score" gorm:"column:teleop_score"`
	EndgameScore     int        `json:"endgame_score" gorm:"column:endgame_score"`
	Notes            *string    `json:"notes,omitempty" gorm:"column:notes"`
	ScouterName      *string    `json:"scouter_name,omitempty" gorm:"column:scouter_name"`
	StartingPosition *string    `json:"starting_position,omitempty" gorm:"column:starting_position"`
	DefenseRating    *string    `json:"defense_rating,omitempty" gorm:"column:defense_rating"`
	Throughput       *string    `json:"throughput,omitempty" gorm:"column:throughput"`
	ScoringStrategy  *string    `json:"scoring_strategy,omitempty" gorm:"column:scoring_strategy"`
	ShootingSpeed    *string    `json:"shooting_speed,omitempty" gorm:"column:shooting_speed"`
	Capacity         *string    `json:"capacity,omitempty" gorm:"column:capacity"`
	Defendability    *string    `json:"defendability,omitempty" gorm:"column:defendability"`
	Traversal        *string    `json:"traversal,omitempty" gorm:"column:traversal"`
	HangLevel        *string    `json:"hang_level,omitempty" gorm:"column:hang_level"`
	AutoHang         *string    `json:"auto_hang,omitempty" gorm:"column:auto_hang"`
	HangPosition     *string    `json:"hang_position,omitempty" gorm:"column:hang_position"`
	HubAutoCount     int        `json:"hub_auto_count" gorm:"column:hub_auto_count"`
	HubTeleopCount   int        `json:"hub_teleop_count" gorm:"column:hub_teleop_count"`
	HubEndgameCount  int        `json:"hub_endgame_count" gorm:"column:hub_endgame_count"`
	PenaltiesCaused  int        `json:"penalties_caused" gorm:"column:penalties_caused"`
	ScoutedAt        *time.Time `json:"scouted_at,omitempty" gorm:"column:scouted_at"`
	ScouterID        *int       `json:"scouter_id,omitempty" gorm:"column:scouter_id"`
	CreatedAt        time.Time  `json:"created_at" gorm:"column:created_at"`
	UpdatedAt        time.Time  `json:"updated_at" gorm:"column:updated_at"`
}

// TeamEventStats represents aggregated team statistics for an event
type TeamEventStats struct {
	ID             int       `json:"id" gorm:"column:id;primaryKey"`
	TeamID         int       `json:"team_id" gorm:"column:team_id"`
	EventID        int       `json:"event_id" gorm:"column:event_id"`
	OPR            *float64  `json:"opr,omitempty" gorm:"column:opr"`
	DPR            *float64  `json:"dpr,omitempty" gorm:"column:dpr"`
	CCWM           *float64  `json:"ccwm,omitempty" gorm:"column:ccwm"`
	AutoOPR        *float64  `json:"auto_opr,omitempty" gorm:"column:auto_opr"`
	TeleopOPR      *float64  `json:"teleop_opr,omitempty" gorm:"column:teleop_opr"`
	EndgameOPR     *float64  `json:"endgame_opr,omitempty" gorm:"column:endgame_opr"`
	Rank           *int      `json:"rank,omitempty" gorm:"column:rank"`
	MatchesPlayed  int       `json:"matches_played" gorm:"column:matches_played"`
	QualAverage    *float64  `json:"qual_average,omitempty" gorm:"column:qual_average"`
	Wins           int       `json:"wins" gorm:"column:wins"`
	Losses         int       `json:"losses" gorm:"column:losses"`
	Ties           int       `json:"ties" gorm:"column:ties"`
	DQCount        int       `json:"dq_count" gorm:"column:dq_count"`
	QualPoints     *int64    `json:"qual_points,omitempty" gorm:"column:qual_points"`
	ElimPoints     *int64    `json:"elim_points,omitempty" gorm:"column:elim_points"`
	AwardPoints    *int64    `json:"award_points,omitempty" gorm:"column:award_points"`
	AlliancePoints *int64    `json:"alliance_points,omitempty" gorm:"column:alliance_points"`
	TotalPoints    *int64    `json:"total_points,omitempty" gorm:"column:total_points"`
	CreatedAt      time.Time `json:"created_at" gorm:"column:created_at"`
	UpdatedAt      time.Time `json:"updated_at" gorm:"column:updated_at"`
}

// TableName specifies the table name for GORM
func (TeamEventStats) TableName() string {
	return "team_event_stats"
}

// TODO: Add more models as needed
// Keep models as plain structs - no ORM magic
// Database operations should be in the db package or handlers
