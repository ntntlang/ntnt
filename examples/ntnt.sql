-- NTNT PostgreSQL Demo Database
-- 
-- To set up the database and user:
--
--   # Create user and database (run as postgres superuser)
--   psql -U postgres -c "CREATE USER ntnt WITH PASSWORD 'password';"
--   psql -U postgres -c "CREATE DATABASE ntnt OWNER ntnt;"
--   psql -U postgres -c "GRANT ALL PRIVILEGES ON DATABASE ntnt TO ntnt;"
--
--   # Import this schema and data
--   psql -U ntnt -d ntnt -f examples/ntnt.sql
--
-- Then run the demo:
--   export DATABASE_URL="postgres://ntnt:password@localhost/ntnt"
--   ./target/release/ntnt run examples/postgres_demo.tnt
--
-- Note: For DECIMAL/NUMERIC columns, use ::float cast in queries
-- e.g., SELECT price::float FROM products

-- Drop existing tables if they exist
DROP TABLE IF EXISTS order_items CASCADE;
DROP TABLE IF EXISTS orders CASCADE;
DROP TABLE IF EXISTS products CASCADE;
DROP TABLE IF EXISTS users CASCADE;

-- Users table
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    age INTEGER,
    active BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Products table
CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(200) NOT NULL,
    description TEXT,
    price DECIMAL(10, 2) NOT NULL,
    stock INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Orders table
CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES users(id),
    status VARCHAR(50) DEFAULT 'pending',
    total DECIMAL(10, 2),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Order items table
CREATE TABLE order_items (
    id SERIAL PRIMARY KEY,
    order_id INTEGER REFERENCES orders(id),
    product_id INTEGER REFERENCES products(id),
    quantity INTEGER NOT NULL,
    price DECIMAL(10, 2) NOT NULL
);

-- Insert sample users
INSERT INTO users (name, email, age, active) VALUES
    ('Alice Smith', 'alice@example.com', 30, true),
    ('Bob Johnson', 'bob@example.com', 25, true),
    ('Charlie Brown', 'charlie@example.com', 35, true),
    ('Diana Ross', 'diana@example.com', 28, false),
    ('Eve Wilson', 'eve@example.com', 32, true);

-- Insert sample products
INSERT INTO products (name, description, price, stock) VALUES
    ('Laptop', 'High-performance laptop with 16GB RAM', 999.99, 50),
    ('Keyboard', 'Mechanical keyboard with RGB lighting', 149.99, 200),
    ('Mouse', 'Wireless ergonomic mouse', 79.99, 150),
    ('Monitor', '27-inch 4K display', 449.99, 75),
    ('Headphones', 'Noise-canceling wireless headphones', 299.99, 100),
    ('Webcam', '1080p HD webcam', 89.99, 120),
    ('USB Hub', '7-port USB 3.0 hub', 39.99, 300),
    ('Desk Lamp', 'LED desk lamp with adjustable brightness', 59.99, 80);

-- Insert sample orders
INSERT INTO orders (user_id, status, total) VALUES
    (1, 'completed', 1149.98),
    (1, 'completed', 79.99),
    (2, 'pending', 449.99),
    (3, 'shipped', 539.97),
    (5, 'completed', 999.99);

-- Insert sample order items
INSERT INTO order_items (order_id, product_id, quantity, price) VALUES
    (1, 1, 1, 999.99),
    (1, 2, 1, 149.99),
    (2, 3, 1, 79.99),
    (3, 4, 1, 449.99),
    (4, 5, 1, 299.99),
    (4, 6, 1, 89.99),
    (4, 2, 1, 149.99),
    (5, 1, 1, 999.99);

-- Create indexes for better query performance
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_active ON users(active);
CREATE INDEX idx_orders_user_id ON orders(user_id);
CREATE INDEX idx_orders_status ON orders(status);
CREATE INDEX idx_order_items_order_id ON order_items(order_id);
CREATE INDEX idx_order_items_product_id ON order_items(product_id);

-- Grant permissions to ntnt user (if running as different user)
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO ntnt;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO ntnt;

-- Verify data
SELECT 'Users: ' || COUNT(*) FROM users;
SELECT 'Products: ' || COUNT(*) FROM products;
SELECT 'Orders: ' || COUNT(*) FROM orders;
SELECT 'Order Items: ' || COUNT(*) FROM order_items;
