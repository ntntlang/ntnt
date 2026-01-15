# NTNT Language Website

The official website for ntnt-lang.org, built with NTNT.

## Running

```bash
# Without database (demo mode)
ntnt run server.tnt

# With PostgreSQL blog
createdb ntnt_blog
psql -d ntnt_blog -f schema.sql
DATABASE_URL=postgres://localhost/ntnt_blog ntnt run server.tnt
```

Visit http://localhost:8080

## Structure

```
ntnt-lang-org/
├── server.tnt           # Main server entry point
├── website.intent       # Intent file for IDD
├── schema.sql           # PostgreSQL schema for blog
├── assets/              # Static files (logos, images)
├── middleware/
│   ├── logger.tnt       # Request logging
│   └── layout.tnt       # HTML layout wrapper
└── routes/
    ├── index.tnt        # Homepage
    ├── learn.tnt        # Learn/documentation page
    └── blog.tnt         # Blog list and post pages
```

## Features

- **Homepage**: Introduction, why NTNT, key features, CTA to GitHub
- **Learn**: Quick start, IDD tutorial, CLI reference, documentation links
- **Blog**: PostgreSQL-backed with markdown rendering (works in demo mode without DB)

## Colors

From the NTNT logo:

- Teal: #137EA2
- Orange: #F1993F
- Indigo: #2C2A74
- Navy: #192F52
- Yellow: #FCDD83
- Cyan: #20DDE3
