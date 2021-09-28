CREATE TABLE "board" (
	"id"         INTEGER GENERATED ALWAYS AS IDENTITY,
	"name"       TEXT NOT NULL,
	"created_at" BIGINT NOT NULL,
	"shape"      TEXT NOT NULL,
	"mask"       BYTEA NOT NULL,
	"initial"    BYTEA NOT NULL,
	PRIMARY KEY ("id")
);

CREATE TABLE "color" (
	"board"   INTEGER NOT NULL,
	"index"   INTEGER NOT NULL,
	"name"    TEXT NOT NULL,
	"value"   INTEGER NOT NULL,
	PRIMARY KEY ("board", "index"),
	FOREIGN KEY ("board") REFERENCES "board"("id")
);

CREATE TABLE "placement"(
	"id"        BIGINT GENERATED ALWAYS AS IDENTITY,
	"board"     INTEGER NOT NULL,
	"position"  BIGINT NOT NULL,
	"color"     SMALLINT NOT NULL,
	"timestamp" INTEGER NOT NULL,
	"user_id"   TEXT NULL,
	PRIMARY KEY ("id"),
	FOREIGN KEY ("board") REFERENCES "board"("id"),
	FOREIGN KEY ("board", "color") REFERENCES "color"("board", "index")
);