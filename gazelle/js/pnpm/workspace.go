package gazelle

import (
	"log"
	"path"

	"github.com/bazelbuild/bazel-gazelle/label"
)

// Convert project paths/names to a common format.
// Often the root project is referenced as ".", other times as blank "".
// This normalizes it to a single format.
func normalizeProject(project string) string {
	if project == "." {
		return ""
	}
	return project
}

// A global map of pnpm projects across the bazel workspace.
// Projects may be from across multiple pnpm workspaces.
type PnpmProjectMap struct {
	projects map[string]*PnpmProject
}

// A pnpm workspace and its projects
type PnpmWorkspace struct {
	pm *PnpmProjectMap

	// The lockfile this struct represents
	lockfile string

	// References to projects. The key is referenced by a project.
	// Currently only needed for IsReferenced() so only a boolean is persisted.
	referenced map[string]bool
}

// A pnpm project and its package dependencies
type PnpmProject struct {
	// The pnpm workspace this project originated from
	workspace *PnpmWorkspace

	// The project path relative to the root
	project string

	// Packages defined in this project and their associated labels
	packages map[string]*label.Label
}

// PnpmProjectMap ----------------------------------------------------------
func NewPnpmProjectMap() *PnpmProjectMap {
	pm := &PnpmProjectMap{}
	pm.projects = make(map[string]*PnpmProject)
	return pm
}
func (pm *PnpmProjectMap) NewWorkspace(lockfile string) *PnpmWorkspace {
	return newPnpmWorkspace(pm, lockfile)
}

func (pm *PnpmProjectMap) addProject(project *PnpmProject) {
	if existing := pm.projects[project.project]; existing != nil {
		log.Fatalf("Project '%s' (workspace: '%s') already exists from '%s'\n", project.project, project.workspace.lockfile, existing.workspace.lockfile)
	}

	pm.projects[project.project] = project
}
func (pm *PnpmProjectMap) GetProject(project string) *PnpmProject {
	for pm.projects[project] == nil {
		if project == "" {
			break
		}

		project = normalizeProject(path.Dir(project))
	}

	return pm.projects[project]
}

func (pm *PnpmProjectMap) IsProject(project string) bool {
	return pm.projects[normalizeProject(project)] != nil
}

func (pm *PnpmProjectMap) IsReferenced(project string) bool {
	p := pm.projects[normalizeProject(project)]

	return p != nil && p.IsReferenced()
}

// PnpmWorkspace ----------------------------------------------------------
func newPnpmWorkspace(pm *PnpmProjectMap, lockfile string) *PnpmWorkspace {
	w := &PnpmWorkspace{}
	w.referenced = make(map[string]bool)
	w.lockfile = lockfile
	w.pm = pm
	return w
}

func (w *PnpmWorkspace) Root() string {
	return path.Dir(w.lockfile)
}

func (w *PnpmWorkspace) AddProject(pkg string) *PnpmProject {
	project := newPnpmProject(w, pkg)
	w.pm.addProject(project)
	return project
}

func (w *PnpmWorkspace) IsReferenced(project string) bool {
	return w.referenced[normalizeProject(project)]
}

func (w *PnpmWorkspace) AddReference(pkg, linkTo string) {
	to := normalizeProject(linkTo)

	if w.pm.projects[to] == nil {
		log.Fatalf("Unknown link '%s' to package '%s' in workspace '%s'\n", linkTo, pkg, w.lockfile)
	}

	w.referenced[to] = true
}

// PnpmProject ----------------------------------------------------------
func newPnpmProject(workspace *PnpmWorkspace, project string) *PnpmProject {
	p := &PnpmProject{}
	p.workspace = workspace
	p.project = normalizeProject(path.Join(workspace.Root(), project))
	p.packages = make(map[string]*label.Label)
	return p
}

func (p *PnpmProject) Pkg() string {
	return p.project
}

func (p *PnpmProject) AddPackage(pkg string, label *label.Label) {
	p.packages[pkg] = label
}

func (p *PnpmProject) Parent() *PnpmProject {
	pp := p.workspace.pm.GetProject(path.Dir(p.project))

	if pp == p {
		return nil
	}

	return pp
}

func (p *PnpmProject) Get(pkg string) *label.Label {
	for pkgProject := p; pkgProject != nil; {
		if found := pkgProject.packages[pkg]; found != nil {
			return found
		}

		pkgProject = pkgProject.Parent()
	}

	return nil
}

func (p *PnpmProject) IsReferenced() bool {
	return p.workspace.IsReferenced(p.project)
}
