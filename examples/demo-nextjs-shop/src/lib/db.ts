import { Client } from "pg";

export async function query(sql: string, params?: any[]) {
  const client = new Client({
    connectionString: process.env.DATABASE_URL,
  });

  await client.connect();

  const result = await client.query(sql, params);

  return result;
}

export async function queryMany(queries: string[]) {
  const results = [];
  for (const sql of queries) {
    results.push(await query(sql));
  }
  return results;
}

export async function rawQuery(sql: string) {
  const client = new Client({
    connectionString: process.env.DATABASE_URL,
  });
  await client.connect();
  const result = await client.query(sql);
  return result;
}
