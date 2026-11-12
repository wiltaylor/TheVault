// Keep the semver suffix out of the numeric patch used for check 49 updates.
export function updateVersions(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-alpha\.([1-9]\d?))?$/.exec(version);
  if (!match || (match[4] && Number(match[4]) > 98)) {
    throw new Error('Invalid release version for update fixtures');
  }
  const [major, minor, patch] = match.slice(1, 4).map(Number);
  return [1, 2].map(bump => `${major}.${minor}.${patch + bump}`);
}
