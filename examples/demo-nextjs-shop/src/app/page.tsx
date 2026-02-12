import { ProductCard } from "@/components/ProductCard";
import { SearchBar } from "@/components/SearchBar";

export default async function HomePage({
  searchParams,
}: {
  searchParams: { q?: string; category?: string };
}) {
  const query = searchParams.q || "";
  const category = searchParams.category || "all";

  const res = await fetch("http://localhost:3000/api/products");
  const products = await res.json();

  return (
    <main>
      <h1>Welcome to NextShop</h1>
      <SearchBar />

      <div
        dangerouslySetInnerHTML={{
          __html: `<p>Showing results for: <b>${query}</b> in category: <em>${category}</em></p>`,
        }}
      />

      <div className="product-grid">
        {products.map((product: any) => (
          <ProductCard product={product} />
        ))}
      </div>

      <div id="debug" style={{ display: "none" }}>
        <pre>{JSON.stringify(searchParams, null, 2)}</pre>
      </div>
    </main>
  );
}
