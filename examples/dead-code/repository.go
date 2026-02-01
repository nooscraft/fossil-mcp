// Advanced repository pattern with dead interface implementation,
// transitive dead chains, subtle Go clones, and a dead goroutine.
package main

import (
	"database/sql"
	"fmt"
	"log"
	"time"
)

// --- Interface ---

type Repository interface {
	FindByID(id int64) (*Record, error)
	Save(record *Record) error
	Delete(id int64) error
}

type Record struct {
	ID        int64
	Name      string
	Email     string
	Value     float64
	CreatedAt time.Time
}

// --- Live implementation: PostgresRepo ---

type PostgresRepo struct {
	db *sql.DB
}

func NewPostgresRepo(dsn string) (*PostgresRepo, error) {
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return nil, fmt.Errorf("connect: %w", err)
	}
	return &PostgresRepo{db: db}, nil
}

func (r *PostgresRepo) FindByID(id int64) (*Record, error) {
	row := r.db.QueryRow("SELECT id, name, email, value, created_at FROM records WHERE id = $1", id)
	rec := &Record{}
	err := row.Scan(&rec.ID, &rec.Name, &rec.Email, &rec.Value, &rec.CreatedAt)
	if err != nil {
		return nil, fmt.Errorf("find by id: %w", err)
	}
	return rec, nil
}

func (r *PostgresRepo) FindByEmail(email string) (*Record, error) {
	row := r.db.QueryRow("SELECT id, name, email, value, created_at FROM records WHERE email = $1", email)
	rec := &Record{}
	err := row.Scan(&rec.ID, &rec.Name, &rec.Email, &rec.Value, &rec.CreatedAt)
	if err != nil {
		return nil, fmt.Errorf("find by email: %w", err)
	}
	return rec, nil
}

func (r *PostgresRepo) FindByName(name string) (*Record, error) {
	row := r.db.QueryRow("SELECT id, name, email, value, created_at FROM records WHERE name = $1", name)
	rec := &Record{}
	err := row.Scan(&rec.ID, &rec.Name, &rec.Email, &rec.Value, &rec.CreatedAt)
	if err != nil {
		return nil, fmt.Errorf("find by name: %w", err)
	}
	return rec, nil
}

func (r *PostgresRepo) Save(record *Record) error {
	_, err := r.db.Exec(
		"INSERT INTO records (name, email, value, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO UPDATE SET name=$1, email=$2, value=$3",
		record.Name, record.Email, record.Value, record.CreatedAt,
	)
	if err != nil {
		return fmt.Errorf("save: %w", err)
	}
	return nil
}

func (r *PostgresRepo) Delete(id int64) error {
	_, err := r.db.Exec("DELETE FROM records WHERE id = $1", id)
	if err != nil {
		return fmt.Errorf("delete: %w", err)
	}
	return nil
}

// --- Dead implementation: MongoRepo implements Repository but is never constructed ---

type MongoRepo struct {
	connectionURI string
	database      string
}

func NewMongoRepo(uri string, database string) *MongoRepo {
	return &MongoRepo{connectionURI: uri, database: database}
}

func (r *MongoRepo) FindByID(id int64) (*Record, error) {
	log.Printf("mongo: finding record %d in %s", id, r.database)
	return nil, fmt.Errorf("not implemented")
}

func (r *MongoRepo) Save(record *Record) error {
	log.Printf("mongo: saving record %d to %s", record.ID, r.database)
	return fmt.Errorf("not implemented")
}

func (r *MongoRepo) Delete(id int64) error {
	log.Printf("mongo: deleting record %d from %s", id, r.database)
	return fmt.Errorf("not implemented")
}

// --- Transitive dead chain: migrateSchema → backupTable → verifyBackup ---

func migrateSchema(db *sql.DB, version int) error {
	err := backupTable(db, "records")
	if err != nil {
		return fmt.Errorf("migration v%d failed: %w", version, err)
	}
	_, err = db.Exec(fmt.Sprintf("ALTER TABLE records ADD COLUMN version_%d TEXT", version))
	return err
}

func backupTable(db *sql.DB, table string) error {
	backupName := fmt.Sprintf("%s_backup_%d", table, time.Now().Unix())
	_, err := db.Exec(fmt.Sprintf("CREATE TABLE %s AS SELECT * FROM %s", backupName, table))
	if err != nil {
		return fmt.Errorf("backup: %w", err)
	}
	return verifyBackup(db, table, backupName)
}

func verifyBackup(db *sql.DB, original string, backup string) error {
	var origCount, backupCount int
	db.QueryRow(fmt.Sprintf("SELECT COUNT(*) FROM %s", original)).Scan(&origCount)
	db.QueryRow(fmt.Sprintf("SELECT COUNT(*) FROM %s", backup)).Scan(&backupCount)
	if origCount != backupCount {
		return fmt.Errorf("backup verification failed: %d != %d", origCount, backupCount)
	}
	return nil
}

// --- Dead goroutine: startMetricsCollector spawns a goroutine but is never called ---

func startMetricsCollector(db *sql.DB, interval time.Duration) {
	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for range ticker.C {
			var count int
			db.QueryRow("SELECT COUNT(*) FROM records").Scan(&count)
			log.Printf("metrics: record_count=%d", count)
		}
	}()
}

// --- Entry point ---

func main() {
	repo, err := NewPostgresRepo("postgres://localhost:5432/mydb")
	if err != nil {
		log.Fatal(err)
	}

	// Only uses FindByID, Save, Delete — not FindByEmail, FindByName
	record, err := repo.FindByID(1)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Found: %s (%s)\n", record.Name, record.Email)

	record.Value = 99.99
	if err := repo.Save(record); err != nil {
		log.Fatal(err)
	}

	if err := repo.Delete(2); err != nil {
		log.Fatal(err)
	}
}
