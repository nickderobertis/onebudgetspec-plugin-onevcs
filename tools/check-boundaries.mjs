// Rust-only graph: the expensive binary suite is a leaf depending only on its build.
import { readFileSync, readdirSync } from 'node:fs';
const projects = readdirSync('crates').map(dir => JSON.parse(readFileSync(`crates/${dir}/project.json`, 'utf8')));
for (const project of projects) {
  if (project.tags.includes('type:e2e')) {
    const app = project.name.replace(/-e2e$/, '');
    if (JSON.stringify(project.implicitDependencies) !== JSON.stringify([app]) ||
        !project.targets.test.dependsOn.includes(`${app}:build`)) {
      throw new Error(`${project.name}: depend only on ${app}, and run after its build`);
    }
  } else if ((project.implicitDependencies ?? []).some(dep => dep.endsWith('-e2e'))) {
    throw new Error(`${project.name}: e2e projects must remain leaves`);
  }
}
