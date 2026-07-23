package store

const postgresCompatibilitySQL = `
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION json_valid(payload bytea)
RETURNS boolean
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
BEGIN
  PERFORM convert_from(payload, 'UTF8')::jsonb;
  RETURN true;
EXCEPTION WHEN others THEN
  RETURN false;
END;
$$;

CREATE OR REPLACE FUNCTION iotkit_json_path(payload bytea, path text)
RETURNS jsonb
LANGUAGE sql
IMMUTABLE
STRICT
AS $$
  SELECT convert_from(payload, 'UTF8')::jsonb #>
    string_to_array(
      regexp_replace(
        regexp_replace(path, '^\$\.', ''),
        '\[([0-9]+)\]', '.\1', 'g'
      ),
      '.'
    );
$$;

CREATE OR REPLACE FUNCTION json_extract(payload bytea, path text)
RETURNS text
LANGUAGE sql
IMMUTABLE
STRICT
AS $$
  SELECT public.iotkit_json_path(payload, path) #>> '{}';
$$;

CREATE OR REPLACE FUNCTION json_type(payload bytea, path text DEFAULT '$')
RETURNS text
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
DECLARE
  value jsonb;
  kind text;
  rendered text;
BEGIN
  IF path = '$' THEN
    value := convert_from(payload, 'UTF8')::jsonb;
  ELSE
    value := public.iotkit_json_path(payload, path);
  END IF;
  kind := jsonb_typeof(value);
  IF kind = 'number' THEN
    rendered := value::text;
    IF rendered ~ '[.eE]' THEN
      RETURN 'real';
    END IF;
    RETURN 'integer';
  END IF;
  RETURN kind;
END;
$$;

CREATE OR REPLACE FUNCTION randomblob(size integer)
RETURNS bytea
LANGUAGE sql
VOLATILE
STRICT
AS $$ SELECT gen_random_bytes(size); $$;

CREATE OR REPLACE FUNCTION hex(payload bytea)
RETURNS text
LANGUAGE sql
IMMUTABLE
STRICT
AS $$ SELECT encode(payload, 'hex'); $$;

CREATE OR REPLACE FUNCTION unixepoch(mode text)
RETURNS double precision
LANGUAGE sql
VOLATILE
STRICT
AS $$ SELECT extract(epoch FROM clock_timestamp()); $$;
`
