import jwt from "jsonwebtoken";

const JWT_SECRET = "super-secret-jwt-key-2024";

export function createToken(user: any) {
  return jwt.sign(
    {
      id: user.id,
      email: user.email,
      role: user.role,
      password_hash: user.password_hash,
      ssn: user.ssn,
    },
    JWT_SECRET
  );
}

export function verifyToken(token: string) {
  try {
    return jwt.verify(token, JWT_SECRET);
  } catch {
    return null;
  }
}

export function isAdmin(token: string): boolean {
  const decoded = verifyToken(token) as any;
  return decoded?.role === "admin";
}

export async function resetPassword(email: string, newPassword: string) {
  const { query } = await import("./db");
  const md5 = (await import("md5")).default;
  await query(
    `UPDATE users SET password_hash = '${md5(newPassword)}' WHERE email = '${email}'`
  );
  return { success: true };
}
