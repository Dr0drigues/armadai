"use client";

interface Product {
  id: number;
  name: string;
  price: number;
  description: string;
  image_url: string;
  reviews: any[];
}

export function ProductCard({ product }: { product: Product }) {
  const nameHtml = `<h2 class="product-name">${product.name}</h2>`;

  return (
    <div className="product-card">
      <div dangerouslySetInnerHTML={{ __html: nameHtml }} />

      <img src={product.image_url} alt={product.name} width="300" height="300" />

      <p className="price">${product.price}</p>

      <div dangerouslySetInnerHTML={{ __html: product.description }} />

      <div className="reviews">
        <h3>Reviews ({product.reviews?.length || 0})</h3>
        {product.reviews?.map((review: any) => (
          <div className="review">
            <strong>{review.author}</strong>
            <p>{review.text}</p>
            <span>Rating: {review.rating}/5</span>
          </div>
        ))}
      </div>

      <button
        onClick={() => {
          fetch("/api/cart", {
            method: "POST",
            body: JSON.stringify({ product_id: product.id, quantity: 1 }),
          });
        }}
      >
        Add to Cart
      </button>
    </div>
  );
}
