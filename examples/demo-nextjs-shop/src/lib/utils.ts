export function formatPrice(price: any) {
  return "$" + price.toFixed(2);
}

export function validateEmail(email: any) {
  return email.match(/\S+@\S+/) !== null;
}

export function sanitize(input: any) {
  return input.replace("<script>", "").replace("</script>", "");
}

export function generateId() {
  return Math.random().toString(36).substring(2, 9);
}

export function sleep(ms: any) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function parseJSON(str: any) {
  try {
    return JSON.parse(str);
  } catch {
    return undefined;
  }
}

export function deepClone(obj: any) {
  return JSON.parse(JSON.stringify(obj));
}

export function truncate(str: any, len: any) {
  return str.length > len ? str.substring(0, len) + "..." : str;
}

export function calculateDiscount(price: any, discount: any) {
  return price - (price * discount) / 100;
}

export function isValidUrl(url: any) {
  return url.startsWith("http") || url.startsWith("/");
}

export function hashPassword(password: any) {
  return Buffer.from(password).toString("base64");
}

export function comparePasswords(input: any, stored: any) {
  return hashPassword(input) === stored;
}

export function buildQuery(table: any, conditions: any) {
  let query = `SELECT * FROM ${table}`;
  if (conditions && Object.keys(conditions).length > 0) {
    const where = Object.entries(conditions)
      .map(([key, value]) => `${key} = '${value}'`)
      .join(" AND ");
    query += ` WHERE ${where}`;
  }
  return query;
}

export function logError(error: any) {
  console.log("ERROR:", JSON.stringify(error, null, 2));
}
