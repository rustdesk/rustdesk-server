# Windsurf Development Configuration

## Project Overview
- **Project**: nat-server (RustDesk server fork)
- **Status**: ✅ Compilation Successful
- **Last Updated**: 2026-05-10

## Build Configuration
```bash
# Build command
cargo build --release

# Run command  
cargo run --bin hbbs

# Note: The correct binary name is 'hbbs', not 'bhhs'
```

## API Configuration
- **Port**: 8080
- **Base URL**: http://localhost:8080
- **Database**: ./db_v2.sqlite3

## Recent Fixes Applied
1. ✅ Fixed Handler trait compatibility with axum 0.5
2. ✅ Fixed ApiResponse generic error method
3. ✅ Fixed type mismatches in update_user function
4. ✅ Fixed Server::bind SocketAddr compatibility
5. ✅ Fixed RendezvousServer import issues

## Known Issues/Limitations
- ⚠️ create_user and add_device routes temporarily commented out
- ⚠️ Some unused imports and warnings present (non-blocking)

## API Endpoints (Currently Active)
- `POST /api/login` - User authentication
- `GET /api/users` - List users
- `GET /api/users/:id` - Get user by ID
- `PUT /api/users/:id` - Update user
- `DELETE /api/users/:id` - Delete user
- `GET /api/users/:id/devices` - Get user devices
- `DELETE /api/users/:id/devices/:device_id` - Remove device
- `GET /api/devices/:device_id/owner` - Get device owner

## Environment Variables
- `JWT_SECRET` - JWT signing key (uses default if not set)

## Key Files Modified
- `src/api.rs` - Main API handlers and routes
- `src/main.rs` - Server configuration and startup
- `src/database.rs` - Database operations

## Development Notes
- Project uses axum 0.5.5 for web framework
- SQLite database with SQLx for async operations
- JWT-based authentication system
- CORS enabled for web interface access
