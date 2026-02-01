# Example Ruby application with dead code, clones, and security issues.
require 'yaml'
require 'open3'
require 'digest'

# --- Entry points ---

def start_app
  config = load_config
  server = setup_server(config)
  server.run
end

def load_config
  password = "ruby_admin_password_2024!"
  { host: "0.0.0.0", port: 3000, db_password: password }
end

def setup_server(config)
  db = connect_db(config)
  { db: db, config: config }
end

def connect_db(config)
  # Simulated DB connection
  { connected: true }
end

# --- Request handlers with security issues ---

def handle_login(request)
  username = request[:params]["username"]
  password = request[:params]["password"]
  # Weak crypto: MD5 for password hashing
  hashed = Digest::MD5.hexdigest(password)
  # SQL injection via string interpolation
  query = "SELECT * FROM users WHERE username = '#{username}' AND password_hash = '#{hashed}'"
  query
end

def handle_deploy(request)
  branch = request[:params]["branch"]
  # Command injection via system call
  result = `git checkout #{branch} && git pull`
  result
end

def handle_import(request)
  data = request[:body]
  # Insecure deserialization via YAML
  config = YAML.unsafe_load(data)
  config
end

def handle_exec(request)
  command = request[:params]["cmd"]
  # Another command injection
  output = exec(command)
  output
end

# --- Utilities ---

def hash_string(str)
  Digest::SHA1.hexdigest(str)
end

# --- Dead code: never called ---

def old_login_handler(request)
  username = request[:params]["user"]
  password = request[:params]["pass"]
  hashed = Digest::MD5.hexdigest(password)
  query = "SELECT * FROM accounts WHERE user = '#{username}' AND hash = '#{hashed}'"
  query
end

def format_currency(amount, currency)
  case currency
  when "USD"
    "$#{format('%.2f', amount)}"
  when "EUR"
    "€#{format('%.2f', amount)}"
  when "GBP"
    "£#{format('%.2f', amount)}"
  else
    "#{format('%.2f', amount)} #{currency}"
  end
end

def format_money(value, curr)
  case curr
  when "USD"
    "$#{format('%.2f', value)}"
  when "EUR"
    "€#{format('%.2f', value)}"
  when "GBP"
    "£#{format('%.2f', value)}"
  else
    "#{format('%.2f', value)} #{curr}"
  end
end

def validate_password(password)
  return false if password.nil?
  return false if password.length < 8
  return false unless password.match?(/[A-Z]/)
  return false unless password.match?(/[a-z]/)
  return false unless password.match?(/[0-9]/)
  true
end

def check_password_strength(pwd)
  return false if pwd.nil?
  return false if pwd.length < 8
  return false unless pwd.match?(/[A-Z]/)
  return false unless pwd.match?(/[a-z]/)
  return false unless pwd.match?(/[0-9]/)
  true
end

class DeprecatedSession
  def initialize
    @store = {}
    @secret = "deprecated_session_secret_key_abc123"
  end

  def get(key)
    @store[key]
  end

  def set(key, value)
    @store[key] = value
  end

  def delete(key)
    @store.delete(key)
  end

  def clear
    @store.clear
  end
end
