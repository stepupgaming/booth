const siteUrl = "https://booth.dev";
const repoUrl = "https://github.com/StepUpGaming/booth";
const apiUrl = "https://api.github.com/repos/StepUpGaming/booth";

export const siteConfig = {
  name: "Booth",
  owner: "StepUpGaming",
  headline: "Free text to speech and voice cloning on your own PC",
  description:
    "Booth by NoFaceTube: free text to speech and voice cloning that runs on your own GPU, with ZONOS3-Mini.",
  links: {
    website: siteUrl,
    github: repoUrl,
    issues: `${repoUrl}/issues`,
    discussions: `${repoUrl}/discussions`,
    releases: `${repoUrl}/releases/latest`,
    license: `${repoUrl}/blob/master/LICENSE`,
    changelog: `${repoUrl}/blob/master/CHANGELOG.md`,
    contributing: `${repoUrl}/blob/master/CONTRIBUTING.md`,
    githubApi: `${apiUrl}/releases?per_page=10`,
  },
};

export type SiteConfig = typeof siteConfig;
