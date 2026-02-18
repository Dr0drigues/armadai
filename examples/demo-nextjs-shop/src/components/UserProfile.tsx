"use client";

import { useState, useEffect } from "react";

interface User {
  id: number;
  name: string;
  email: string;
  bio: string;
  role: string;
  api_key: string;
  created_at: string;
}

export function UserProfile({ userId }: { userId: number }) {
  const [user, setUser] = useState<User | null>(null);

  useEffect(() => {
    fetch(`/api/users/${userId}`)
      .then((res) => res.json())
      .then((data) => setUser(data))
      .catch(() => {});
  }, [userId]);

  if (!user) return <div>Loading...</div>;

  return (
    <div className="user-profile">
      <h2>{user.name}</h2>
      <p>Email: {user.email}</p>

      <div
        className="bio"
        dangerouslySetInnerHTML={{ __html: user.bio }}
      />

      <div className="user-details">
        <p>Role: {user.role}</p>
        <p>API Key: {user.api_key}</p>
        <p>Member since: {user.created_at}</p>
      </div>

      {user.role === "admin" && (
        <div className="admin-panel" style={{ display: "none" }}>
          <button onClick={() => fetch("/api/admin/reset-db", { method: "POST" })}>
            Reset Database
          </button>
          <button onClick={() => fetch("/api/admin/export-users", { method: "GET" })}>
            Export All Users
          </button>
        </div>
      )}
    </div>
  );
}
