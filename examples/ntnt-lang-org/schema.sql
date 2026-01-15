-- Schema for ntnt-lang.org blog
-- Run: psql -d ntnt_blog -f schema.sql

CREATE TABLE IF NOT EXISTS blog_posts (
    id SERIAL PRIMARY KEY,
    slug VARCHAR(255) UNIQUE NOT NULL,
    title VARCHAR(500) NOT NULL,
    excerpt TEXT,
    content TEXT NOT NULL,
    author VARCHAR(255) DEFAULT 'NTNT Team',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Index for fast slug lookups
CREATE INDEX IF NOT EXISTS idx_blog_posts_slug ON blog_posts(slug);
CREATE INDEX IF NOT EXISTS idx_blog_posts_created_at ON blog_posts(created_at DESC);

-- Example post
INSERT INTO blog_posts (slug, title, excerpt, content, author) VALUES
(
    'welcome',
    'Welcome to the NTNT Blog',
    'Introducing the NTNT programming language and Intent-Driven Development.',
    '# Welcome to the NTNT Blog

Welcome to the official blog for the NTNT programming language!

NTNT (pronounced "Intent") is an experimental programming language designed for AI-assisted software development. It introduces Intent-Driven Development (IDD), where human requirements become executable specifications that AI agents implement and the system verifies.

## What Makes NTNT Different?

Traditional programming languages were designed for humans typing code character by character. As AI agents become capable of generating software, the requirements shift:

- **Agents need structured validation** - machine-readable contracts and introspection
- **Humans need intent clarity** - understand what code should do without reading every line
- **Both need traceability** - connect requirements to implementation

## Getting Started

Check out the Learn page to get started with NTNT, or visit our GitHub repository for the source code and documentation.

Remember: NTNT is experimental and not ready for production use.',
    'NTNT Team'
) ON CONFLICT (slug) DO NOTHING;
