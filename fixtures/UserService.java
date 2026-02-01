import java.io.*;
import java.sql.*;
import java.security.MessageDigest;

/**
 * Example Java service with dead code, clones, and security issues.
 */
public class UserService {

    private Connection db;
    private String apiKey = "sk-prod-java-api-key-1234567890abcdef";

    public UserService(Connection db) {
        this.db = db;
    }

    // --- Active methods ---

    public User findUser(String userId) throws Exception {
        // SQL Injection: string concatenation in query
        String sql = "SELECT * FROM users WHERE id = '" + userId + "'";
        Statement stmt = db.createStatement();
        ResultSet rs = stmt.executeQuery(sql);
        if (rs.next()) {
            return new User(rs.getString("name"), rs.getString("email"));
        }
        return null;
    }

    public void updateUser(String userId, String name) throws Exception {
        String sql = "UPDATE users SET name = '" + name + "' WHERE id = '" + userId + "'";
        Statement stmt = db.createStatement();
        stmt.execute(sql);
    }

    public String hashPassword(String password) throws Exception {
        // Weak crypto: MD5
        MessageDigest md = MessageDigest.getInstance("MD5");
        byte[] hash = md.digest(password.getBytes());
        return bytesToHex(hash);
    }

    public String hashToken(String token) throws Exception {
        // Weak crypto: SHA1
        MessageDigest md = MessageDigest.getInstance("SHA1");
        byte[] hash = md.digest(token.getBytes());
        return bytesToHex(hash);
    }

    public Object loadUserData(String filePath) throws Exception {
        // Insecure deserialization
        FileInputStream fis = new FileInputStream(filePath);
        ObjectInputStream ois = new ObjectInputStream(fis);
        return ois.readObject();
    }

    public String readUserFile(HttpServletRequest request) throws Exception {
        // Path traversal
        String path = request.getParameter("file");
        FileInputStream fis = new FileInputStream("/data/" + request.getParameter("path"));
        return new String(fis.readAllBytes());
    }

    // --- Utility ---

    private String bytesToHex(byte[] bytes) {
        StringBuilder sb = new StringBuilder();
        for (byte b : bytes) {
            sb.append(String.format("%02x", b));
        }
        return sb.toString();
    }

    // --- Dead code: never called ---

    public void migrateUsers() throws Exception {
        String sql = "ALTER TABLE users ADD COLUMN last_login TIMESTAMP";
        Statement stmt = db.createStatement();
        stmt.execute(sql);
    }

    public void exportUsers(String outputPath) throws Exception {
        String sql = "SELECT * FROM users";
        Statement stmt = db.createStatement();
        ResultSet rs = stmt.executeQuery(sql);
        PrintWriter writer = new PrintWriter(outputPath);
        while (rs.next()) {
            writer.println(rs.getString("name") + "," + rs.getString("email"));
        }
        writer.close();
    }

    private String formatUserName(String first, String last) {
        if (first == null || first.isEmpty()) {
            return last;
        }
        if (last == null || last.isEmpty()) {
            return first;
        }
        return first + " " + last;
    }

    private String formatDisplayName(String firstName, String lastName) {
        if (firstName == null || firstName.isEmpty()) {
            return lastName;
        }
        if (lastName == null || lastName.isEmpty()) {
            return firstName;
        }
        return firstName + " " + lastName;
    }

    class DeprecatedAuthProvider {
        private String secretKey = "deprecated_auth_secret_key_12345678";

        public boolean authenticate(String token) {
            return token.equals(secretKey);
        }

        public String generateToken(String userId) {
            return userId + ":" + secretKey;
        }
    }
}
