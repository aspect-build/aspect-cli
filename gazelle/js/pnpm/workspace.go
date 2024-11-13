package gazelle

import (
	"log"
	"path"
	"strings"

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

	// Packages defined in this project which reference other content
	// accessible in the workspace source.
	references map[string]string

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

	return p.workspace.IsReferenced(p.project)
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

// PnpmProject ----------------------------------------------------------
func newPnpmProject(workspace *PnpmWorkspace, project string) *PnpmProject {
	p := &PnpmProject{}
	p.workspace = workspace
	p.project = normalizeProject(path.Join(workspace.Root(), project))
	p.packages = make(map[string]*label.Label)
	p.references = make(map[string]string)
	return p
}

func (p *PnpmProject) Pkg() string {
	return p.project
}

func (p *PnpmProject) addLocalReference(pkg, dir string) {
	// Persist the directory which this local package references
	p.references[pkg] = dir

	// Flag the directory as one that is referenced (assuming? it is also a project)
	p.workspace.referenced[normalizeProject(dir)] = true
}

func (p *PnpmProject) GetLocalReference(pkg string) (string, bool) {
	dir, found := p.references[pkg]
	return dir, found
}

func (p *PnpmProject) AddPackage(pkg, version string, label *label.Label) {
	p.packages[pkg] = label

	// If this is a local workspace link or file reference normalize the path and collect the references
	if strings.HasPrefix(version, "link:") {
		link := version[len("link:"):]

		// Pnpm "link" references are relative to the package defining the link
		p.addLocalReference(pkg, path.Join(p.Pkg(), link))
	} else if strings.HasPrefix(version, "file:") {
		file := version[len("file:"):]

		// Pnpm "file" references are relative to the pnpm workspace root.
		p.addLocalReference(pkg, path.Join(path.Dir(p.workspace.lockfile), file))
	}
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
