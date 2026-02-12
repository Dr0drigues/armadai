import { NextRequest, NextResponse } from "next/server";
import { query } from "@/lib/db";

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { items, payment_method, shipping_address } = body;

  let totalPrice = 0;

  for (const item of items) {
    totalPrice += item.price * item.quantity;

    const stockResult = await query(
      `SELECT stock FROM products WHERE id = ${item.product_id}`
    );

    const currentStock = stockResult.rows[0]?.stock || 0;

    if (currentStock < item.quantity) {
      return NextResponse.json(
        { error: `Insufficient stock for product ${item.product_id}` },
        { status: 400 }
      );
    }

    await query(
      `UPDATE products SET stock = stock - ${item.quantity} WHERE id = ${item.product_id}`
    );
  }

  const order = await query(
    `INSERT INTO orders (total_price, payment_method, shipping_address, status)
     VALUES (${totalPrice}, '${payment_method}', '${shipping_address}', 'confirmed')
     RETURNING *`
  );

  console.log("Order placed:", {
    orderId: order.rows[0].id,
    payment_method,
    totalPrice,
    card_details: body.card,
  });

  return NextResponse.json({
    order: order.rows[0],
    message: "Order confirmed!",
  });
}
