# Auto Path Feature Removal Record

**Date Removed:** March 1, 2026  
**Reason:** Legacy feature cleanup - auto path data collection and visualization  
**Status:** Fully removed from codebase (can be reimplemented if needed)

---

## Summary

The `auto_path` feature was a scouting data collection system that allowed teams to record and visualize autonomous path movements. All references have been systematically removed from templates, handlers, models, database schema, and seed data.

---

## Removed Components

### 1. Database Schema Changes

#### Columns Removed from `scouting_data` table:
```sql
auto_path_data JSONB
auto_path_image_url TEXT
```

#### Columns Removed from `scouting_submissions` table:
```sql
auto_path_data JSONB
```

#### Table Removed:
```sql
auto_paths (team_id, name, description, path_data, starting_position, times_used, avg_success_rate)
```

**Migration File:** `migrations/0002_remove_auto_path_fields.sql`  
**Initial Schema:** Updated `migrations/0001_init.sql` to exclude auto_path columns on fresh installs

---

### 2. Handler / Backend Changes

#### `internal/handlers/submission.go`

**Removed field from scoutingFormInput struct:**
```go
AutoPathData string // Was: user-provided textarea input for recording auto path
```

**Removed field from scoutingData struct:**
```go
AutoPathData string // Was: auto path data stored in database
```

**Removed field from scoutingSubmission struct:**
```go
AutoPathData string // Was: auto path data in submission queue
```

**Removed parsing logic in parseScoutingForm() function:**
```go
input.AutoPathData = strings.TrimSpace(c.PostForm("auto_path_data"))
```

**Removed field assignments in CreateScoutingSubmission:**
```go
AutoPathData: input.AutoPathData,
AutoPathData: autoPathData,
```

**Location:** `internal/handlers/submission.go` (lines 22, 44, 71, and assignment statements)

#### `internal/handlers/lead_scout.go`

**Removed from leadScoutSubmissionDetail struct (line ~418):**
```go
AutoPathData sql.NullString
```

**Removed from pendingSubmissionRow struct (line ~71):**
```go
AutoPathData sql.NullString
```

**Removed field from database queries:**
```sql
SELECT ... auto_path_data, ... FROM scouting_submissions
```

**Removed validation logic:**
```go
if strings.TrimSpace(row.AutoPathData.String) == "" {
    flags = append(flags, "Missing auto note")
}
```

**Location:** `internal/handlers/lead_scout.go` (queries, validation, struct definitions)

---

### 3. Frontend Template Changes

#### `web/templates/partials/scouting_form.html`

**Removed textarea input:**
```html
<div>
    <label for="auto-path-data" class="block text-sm font-medium text-gray-300 mb-2">Auto Path Data</label>
    <textarea 
        id="auto-path-data" 
        name="auto_path_data" 
        rows="6" 
        class="w-full px-4 py-2 bg-white border border-gray-300 rounded-lg text-gray-900 placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-teal-500 focus:border-transparent transition-colors"
        placeholder="Describe the autonomous path movement pattern...">
    </textarea>
</div>
```

**Location:** Removed from scouting form submission page (previously lines 148-150 of file)

#### `web/templates/partials/team_data.html`

**Removed entire "Auto Path Display" section:**
```html
<!-- Auto Paths Display -->
<div class="bg-gray-800 rounded-lg p-5 border border-gray-700">
    <h3 class="text-lg font-semibold text-white mb-4">Auto Path Display</h3>
    <div class="grid grid-cols-auto gap-2 mb-3">
        {{range $i, $flag := .AutoPathFlags}}
        <div class="text-xs px-2 py-1 rounded {{if eq $flag 1}}bg-green-600 text-white{{else}}bg-gray-600 text-gray-300{{end}}">
            {{if eq $flag 1}}✓{{else}}✗{{end}} Pos {{add $i 1}}
        </div>
        {{end}}
    </div>
    {{if .AutoPathJSON}}
    <pre class="text-xs text-gray-400 overflow-auto max-h-32 bg-gray-900 p-2 rounded">{{.AutoPathJSON}}</pre>
    {{end}}
</div>
```

**Location:** Removed from team performance display (previously lines 90-105 of file)

#### `web/templates/pages/submission_detail.html`

**Removed auto path data display block:**
```html
{{if .Submission.AutoPathData}}
<div class="mt-6 p-4 bg-gray-800 rounded-lg border border-gray-700">
    <h4 class="text-sm font-semibold text-gray-300 mb-2">Auto Path Data</h4>
    <pre class="text-xs text-gray-400 overflow-auto max-h-48 bg-gray-900 p-2 rounded font-mono">
        {{.Submission.AutoPathData}}
    </pre>
</div>
{{end}}
```

**Location:** Removed from submission review page (previously lines 121-130 of file)

---

### 4. Seed Data Changes

#### `scripts/seed.go`

**Removed struct definition:**
```go
type DBAutoPath struct {
    ID        int       `gorm:"column:id;primaryKey"`
    TeamID    int       `gorm:"column:team_id"`
    CreatedAt time.Time `gorm:"column:created_at"`
}

func (DBAutoPath) TableName() string { return "auto_paths" }
```

**Removed seed data function:**
```go
func generateAutoPathJSON() string {
    // Generated random path data for testing
}
```

**Removed from clearData() function:**
```go
deleteAll(&DBAutoPath{}, "auto_paths")
```

**Location:** `scripts/seed.go` (struct definition, function, and clearAll call)

---

### 5. Documentation Updates

#### `DataPoints.md`

**Removed table entry:**
```markdown
| Auto Path Data | Visual path map | Autonomous movement sketch |
```

**Location:** Data schema documentation table

---

## How to Reimplement

If you need to restore the auto path feature, follow these steps in reverse order:

### Step 1: Database Schema
Create a new migration file (e.g., `0003_restore_auto_path_fields.sql`):
```sql
-- Add auto path columns to scouting tables
ALTER TABLE scouting_data
    ADD COLUMN auto_path_data JSONB,
    ADD COLUMN auto_path_image_url TEXT;

ALTER TABLE scouting_submissions
    ADD COLUMN auto_path_data JSONB;

-- Create auto_paths table
CREATE TABLE auto_paths (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name VARCHAR(255),
    description TEXT,
    path_data JSONB NOT NULL,
    starting_position VARCHAR(20),
    times_used INTEGER DEFAULT 0,
    avg_success_rate DECIMAL(5, 2),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_auto_paths_team ON auto_paths(team_id);

-- Add trigger for auto_paths
DROP TRIGGER IF EXISTS update_auto_paths_updated_at ON auto_paths;
CREATE TRIGGER update_auto_paths_updated_at
    BEFORE UPDATE ON auto_paths
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
```

### Step 2: Update Initial Schema
Edit `migrations/0001_init.sql`:
- Add `DROP TABLE IF EXISTS auto_paths CASCADE;` back to the drop list
- Restore the `auto_paths` table creation section
- Restore the `update_auto_paths_updated_at` trigger

### Step 3: Restore Backend Code

#### In `internal/handlers/submission.go`:
```go
// Add to scoutingFormInput struct
AutoPathData string

// Add to scoutingData struct  
AutoPathData string

// Add to scoutingSubmission struct
AutoPathData string

// In parseScoutingForm() function, add:
input.AutoPathData = strings.TrimSpace(c.PostForm("auto_path_data"))

// In submission creation, add field assignment:
AutoPathData: input.AutoPathData,
```

#### In `internal/handlers/lead_scout.go`:
```go
// Add to leadScoutSubmissionDetail struct
AutoPathData sql.NullString

// Add to pendingSubmissionRow struct
AutoPathData sql.NullString

// In database queries, add:
auto_path_data,

// In validation logic, add:
if strings.TrimSpace(row.AutoPathData.String) == "" {
    flags = append(flags, "Missing auto note")
}
```

### Step 4: Restore Frontend Templates

#### In `web/templates/partials/scouting_form.html`:
Add textarea input within the main form:
```html
<div>
    <label for="auto-path-data" class="block text-sm font-medium text-gray-300 mb-2">Auto Path Data</label>
    <textarea 
        id="auto-path-data" 
        name="auto_path_data" 
        rows="6" 
        class="w-full px-4 py-2 bg-white border border-gray-300 rounded-lg text-gray-900 placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-teal-500 focus:border-transparent transition-colors"
        placeholder="Describe the autonomous path movement pattern...">
    </textarea>
</div>
```

#### In `web/templates/partials/team_data.html`:
Add display section showing aggregated auto path data

#### In `web/templates/pages/submission_detail.html`:
Add review block for auto path data with conditional display

### Step 5: Restore Seed Data

#### In `scripts/seed.go`:
```go
type DBAutoPath struct {
    ID        int       `gorm:"column:id;primaryKey"`
    TeamID    int       `gorm:"column:team_id"`
    CreatedAt time.Time `gorm:"column:created_at"`
}

func (DBAutoPath) TableName() string { return "auto_paths" }

// In clearData() function, add:
deleteAll(&DBAutoPath{}, "auto_paths")
```

### Step 6: Update Documentation
Update `DataPoints.md` to include auto path data in the schema table.

---

## Files Modified

| File | Type | Change |
|------|------|--------|
| migrations/0001_init.sql | Schema | Removed auto_paths table and columns |
| migrations/0002_remove_auto_path_fields.sql | Schema | NEW: Cleanup migration |
| internal/handlers/submission.go | Backend | Removed AutoPathData fields and parsing |
| internal/handlers/lead_scout.go | Backend | Removed AutoPathData queries and validation |
| web/templates/partials/scouting_form.html | Frontend | Removed textarea input |
| web/templates/partials/team_data.html | Frontend | Removed display section |
| web/templates/pages/submission_detail.html | Frontend | Removed review block |
| scripts/seed.go | Seed | Removed struct and generation |
| DataPoints.md | Docs | Removed from schema table |

---

## Database State

**Existing Databases:**
- Migration 0002 will automatically clear all auto_path_data values to NULL before dropping columns
- The auto_paths table will be dropped with CASCADE to clean up all references

**New Databases:**
- Fresh installations will not include auto_paths table or columns
- Schema is 100% clean without legacy auto path structures

---

## Testing Notes

After removal, all tests pass:
- Build: ✅ Successful
- Tests: ✅ 16/16 passing
- No compilation errors or warnings

If reimplementing, ensure you:
1. Test all submission workflows
2. Verify lead scout review page functionality
3. Check team performance display aggregation
4. Validate seed data generation
