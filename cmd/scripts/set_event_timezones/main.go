package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
)

// Map US states to their primary IANA timezone
var stateTimezones = map[string]string{
	// Eastern Time
	"CT": "America/New_York", "DE": "America/New_York", "FL": "America/New_York",
	"GA": "America/New_York", "MA": "America/New_York", "MD": "America/New_York",
	"ME": "America/New_York", "NC": "America/New_York", "NH": "America/New_York",
	"NJ": "America/New_York", "NY": "America/New_York", "OH": "America/New_York",
	"PA": "America/New_York", "RI": "America/New_York", "SC": "America/New_York",
	"VA": "America/New_York", "VT": "America/New_York", "WV": "America/New_York",
	"MI": "America/Detroit", // Eastern but separate zone

	// Central Time
	"AL": "America/Chicago", "AR": "America/Chicago", "IL": "America/Chicago",
	"IA": "America/Chicago", "KS": "America/Chicago", "LA": "America/Chicago",
	"MN": "America/Chicago", "MS": "America/Chicago", "MO": "America/Chicago",
	"NE": "America/Chicago", "OK": "America/Chicago", "SD": "America/Chicago",
	"TN": "America/Chicago", "TX": "America/Chicago", "WI": "America/Chicago",
	"IN": "America/Indiana/Indianapolis", // Most of Indiana is Eastern, some Central

	// Mountain Time
	"AZ": "America/Phoenix", // Arizona doesn't observe DST
	"CO": "America/Denver", "MT": "America/Denver", "NM": "America/Denver",
	"UT": "America/Denver", "WY": "America/Denver", "ID": "America/Boise",

	// Pacific Time
	"CA": "America/Los_Angeles", "NV": "America/Los_Angeles",
	"OR": "America/Los_Angeles", "WA": "America/Los_Angeles",

	// Alaska & Hawaii
	"AK": "America/Anchorage",
	"HI": "Pacific/Honolulu",

	// Canadian provinces (common FRC regions)
	"ON": "America/Toronto", "QC": "America/Montreal",
	"BC": "America/Vancouver", "AB": "America/Edmonton",
	"SK": "America/Regina", "MB": "America/Winnipeg",
	"NS": "America/Halifax", "NB": "America/Moncton",
}

func main() {
	dsn := os.Getenv("DATABASE_URL")
	if dsn == "" {
		log.Fatal("DATABASE_URL environment variable not set")
	}

	db, err := gorm.Open(postgres.Open(dsn), &gorm.Config{})
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}

	var events []struct {
		ID       int
		Name     string
		Location *string
		Timezone *string
	}

	if err := db.Table("events").
		Select("id, name, location, timezone").
		Where("timezone IS NULL OR timezone = ''").
		Find(&events).Error; err != nil {
		log.Fatalf("Failed to query events: %v", err)
	}

	fmt.Printf("Found %d events without timezone\n\n", len(events))

	ctx := context.Background()
	updated := 0
	skipped := 0

	for _, event := range events {
		if event.Location == nil || *event.Location == "" {
			fmt.Printf("⏭️  Skipping event %d (%s): No location\n", event.ID, event.Name)
			skipped++
			continue
		}

		loc := strings.ToUpper(strings.TrimSpace(*event.Location))

		// Try to extract state abbreviation from location
		// Common formats: "City, ST USA" or "City, ST"
		var timezone string
		for abbr, tz := range stateTimezones {
			if strings.Contains(loc, ", "+abbr) || strings.Contains(loc, " "+abbr+" ") || strings.HasSuffix(loc, abbr) {
				timezone = tz
				break
			}
		}

		if timezone == "" {
			fmt.Printf("⚠️  Could not determine timezone for event %d (%s) - Location: %s\n",
				event.ID, event.Name, *event.Location)
			skipped++
			continue
		}

		// Update the event
		if err := db.WithContext(ctx).
			Table("events").
			Where("id = ?", event.ID).
			Update("timezone", timezone).Error; err != nil {
			fmt.Printf("❌ Failed to update event %d: %v\n", event.ID, err)
			continue
		}

		fmt.Printf("✅ Updated event %d (%s): %s → %s\n",
			event.ID, event.Name, *event.Location, timezone)
		updated++
	}

	fmt.Printf("\n📊 Summary:\n")
	fmt.Printf("   Updated: %d\n", updated)
	fmt.Printf("   Skipped: %d\n", skipped)
	fmt.Printf("   Total:   %d\n", len(events))
}
