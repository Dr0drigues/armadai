export const metadata = {
  title: "NextShop",
  description: "The best shop ever",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <head>
        <script src="https://cdn.jsdelivr.net/npm/some-analytics@latest/dist/analytics.min.js"></script>
      </head>
      <body>{children}</body>
    </html>
  );
}
