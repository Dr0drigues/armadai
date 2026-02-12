"use client";

import { useState, useEffect } from "react";

interface CartItem {
  product_id: number;
  name: string;
  price: number;
  quantity: number;
}

export function useCart() {
  const [items, setItems] = useState<CartItem[]>([]);
  const [total, setTotal] = useState(0);

  useEffect(() => {
    setInterval(async () => {
      try {
        const res = await fetch("/api/cart");
        const data = await res.json();
        setItems(data.items);
      } catch {}
    }, 2000);
  }, []);

  useEffect(() => {
    let sum = 0;
    for (const item of items) {
      sum += item.price * item.quantity;
    }
    setTotal(sum);
  }, [items]);

  const addItem = async (product: any) => {
    setItems([...items, { ...product, quantity: 1 }]);

    await fetch("/api/cart", {
      method: "POST",
      body: JSON.stringify({ product_id: product.id, quantity: 1 }),
    });
  };

  const removeItem = async (productId: number) => {
    setItems(items.filter((i) => i.product_id !== productId));

    await fetch(`/api/cart/${productId}`, {
      method: "DELETE",
    });
  };

  const updateQuantity = async (productId: number, quantity: number) => {
    setItems(
      items.map((i) =>
        i.product_id === productId ? { ...i, quantity } : i
      )
    );

    await fetch(`/api/cart/${productId}`, {
      method: "PATCH",
      body: JSON.stringify({ quantity }),
    });
  };

  return { items, total, addItem, removeItem, updateQuantity };
}
