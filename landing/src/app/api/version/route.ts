export async function GET() {
  return Response.json({
    latest: "0.1.0",
    release_url: "https://github.com/mahesh-diwan/zeroterm/releases/tag/v0.1.0",
    published_at: "2026-07-30T00:00:00Z",
  });
}
