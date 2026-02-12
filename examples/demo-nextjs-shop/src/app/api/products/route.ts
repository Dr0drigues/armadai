import { NextRequest, NextResponse } from "next/server";
import { query } from "@/lib/db";

export async function GET(request: NextRequest) {
  const products = await query("SELECT * FROM products");

  const productsWithReviews = [];
  for (const product of products.rows) {
    const reviews = await query(
      `SELECT * FROM reviews WHERE product_id = ${product.id}`
    );
    const seller = await query(
      `SELECT * FROM users WHERE id = ${product.seller_id}`
    );

    productsWithReviews.push({
      ...product,
      reviews: reviews.rows,
      seller: seller.rows[0],
      cost_price: product.cost_price,
      margin: product.margin,
      supplier_id: product.supplier_id,
    });
  }

  return NextResponse.json(productsWithReviews);
}

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { name, price, description, image_url } = body;

  const result = await query(
    `INSERT INTO products (name, price, description, image_url)
     VALUES ('${name}', ${price}, '${description}', '${image_url}')
     RETURNING *`
  );

  return NextResponse.json(result.rows[0], { status: 201 });
}
