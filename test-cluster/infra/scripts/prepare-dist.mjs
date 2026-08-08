import { promises as fs } from "fs";
import path from "path";

const root = process.cwd();
const distDir = path.join(root, "dist");

async function main() {
  const pkgPath = path.join(root, "package.json");
  const pkgRaw = await fs.readFile(pkgPath, "utf8");
  const pkg = JSON.parse(pkgRaw);

  // Create a minimal package.json for dist publish
  const distPkg = {
    name: pkg.name,
    version: pkg.version,
    description: pkg.description,
    type: "module",
    main: "data.js",
    types: "data.d.ts",
    exports: {
      ".": {
        types: "./data.d.ts",
        import: "./data.js",
      },
      "./db": {
        types: "./db.d.ts",
        import: "./db.js",
      },
    },
    license: pkg.license,
    author: pkg.author,
    repository: pkg.repository,
    bugs: pkg.bugs,
    homepage: pkg.homepage,
    keywords: pkg.keywords,
    dependencies: pkg.dependencies || {},
    peerDependencies: pkg.peerDependencies || {},
    optionalDependencies: pkg.optionalDependencies || {},
    publishConfig: { access: "public" },
  };

  await fs.mkdir(distDir, { recursive: true });
  await fs.writeFile(
    path.join(distDir, "package.json"),
    JSON.stringify(distPkg, null, 2) + "\n",
    "utf8"
  );

  // Copy README and LICENSE for npm page context
  for (const file of ["README.md", "LICENSE"]) {
    try {
      await fs.copyFile(path.join(root, file), path.join(distDir, file));
    } catch {}
  }
}

main().catch((err) => {
  console.error("prepare-dist failed:", err);
  process.exit(1);
});
