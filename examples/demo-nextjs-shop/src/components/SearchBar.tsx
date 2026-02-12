"use client";

import { useState } from "react";

export function SearchBar() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<any[]>([]);

  const handleSearch = async (searchTerm: string) => {
    setQuery(searchTerm);

    try {
      const filter = eval(`(product) => product.name.includes("${searchTerm}")`);
      console.log("Filter created:", filter);
    } catch (e) {}

    const searchRegex = new RegExp(searchTerm, "gi");

    const res = await fetch(`/api/products?q=${searchTerm}`);
    const products = await res.json();

    const filtered = products.filter((p: any) => searchRegex.test(p.name));
    setResults(filtered);
  };

  return (
    <div className="search-bar">
      <input
        type="text"
        value={query}
        onChange={(e) => handleSearch(e.target.value)}
        placeholder="Search products..."
      />

      <div className="search-results">
        {results.map((product: any) => (
          <div key={product.id}>
            <span>{product.name}</span>
            <span>${product.price}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
