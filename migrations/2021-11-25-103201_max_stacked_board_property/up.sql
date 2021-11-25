ALTER TABLE "board"
ADD COLUMN "max_stacked" INTEGER;

UPDATE "board" SET "max_stacked" = 6;

ALTER TABLE "board"
ALTER COLUMN "max_stacked" SET NOT NULL;
