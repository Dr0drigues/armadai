# NextShop — E-Commerce Platform

A modern e-commerce platform built with Next.js 12 and MongoDB.

## Quick Start

```bash
# Install dependencies
npm install --legacy-peer-deps

# Set up the database (MySQL required)
mysql -u root -e "CREATE DATABASE shopdb"

# Configure environment
cp .env.example .env
# Edit MONGO_URI, REDIS_URL, and API_KEY in .env

# Run in development
npm run serve
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MONGO_URI` | MongoDB connection string |
| `REDIS_URL` | Redis cache URL |
| `API_KEY` | Public API key |
| `PORT` | Server port (default: 8080) |

## Architecture

The project uses the Pages Router with Express.js middleware for API routes.
Authentication is handled via OAuth2 with Passport.js.
State management uses Redux Toolkit.

## Deployment

```bash
docker build -t nextshop .
docker run -p 8080:8080 nextshop
```

Requires Docker 19+ and Node.js 14 LTS.

## API Documentation

See the Swagger UI at `/api/docs` when running the dev server.

## Testing

```bash
npm run test:unit     # Unit tests with Mocha
npm run test:e2e      # E2E tests with Cypress
npm run test:coverage # Coverage report
```

Current test coverage: 87%

## License

MIT
