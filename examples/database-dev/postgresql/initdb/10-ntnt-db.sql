-- Runs once, when the container first initializes its data directory.
-- Local development only: fixed throwaway passwords on a loopback-only port.
--
-- `ntnt db` needs the target database and a separate, empty scratch database
-- for disposable replay, each owned by its own unprivileged role. Both are
-- created from template0 with the cluster's defaults, so their settings match.
-- The manifest manages the `public` schema, which each database's owner owns.
CREATE ROLE app_owner LOGIN PASSWORD 'dev-only-owner';
CREATE DATABASE app_development OWNER app_owner TEMPLATE template0;
CREATE ROLE app_replay_owner LOGIN PASSWORD 'dev-only-replay';
CREATE DATABASE app_replay OWNER app_replay_owner TEMPLATE template0;
