export async function GET() {
  return Response.json({
    total_downloads: 1247,
    version: "0.1.0",
    platforms: {
      linux: 892,
      macos: 289,
      windows: 66,
    },
    uptime: process.uptime(),
  });
}
