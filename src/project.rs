//! Project management functionality for multi-project support.
//!
//! This module handles project discovery, naming conventions, and project-specific
//! database file management. Projects are stored as individual JSON files with
//! the naming convention: `<project_name>_tasks.json`.

use std::path::{Path, PathBuf};
use std::fs;
use crate::db::Database;

/// Represents a project with its name and database file path.
#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    pub display_name: String,
    pub file_path: PathBuf,
}

impl Project {
    /// Create a new project with the given display name.
    pub fn new(display_name: &str, pm_dir: &Path) -> Self {
        let name = sanitize_project_name(display_name);
        let file_path = pm_dir.join(format!("{}_tasks.json", name));
        
        Project {
            name,
            display_name: display_name.to_string(),
            file_path,
        }
    }
    
    /// Load a project from an existing database file.
    pub fn from_file(file_path: PathBuf) -> Option<Self> {
        let file_name = file_path.file_stem()?.to_str()?;
        
        if !file_name.ends_with("_tasks") {
            return None;
        }
        
        let name = file_name.strip_suffix("_tasks")?;
        let display_name = name.replace('_', " ");
        
        Some(Project {
            name: name.to_string(),
            display_name,
            file_path,
        })
    }
    
    /// Create the database file for this project if it doesn't exist.
    pub fn create_if_not_exists(&self) -> Result<(), std::io::Error> {
        if !self.file_path.exists() {
            let db = Database::default();
            db.save(&self.file_path)?;
        }
        Ok(())
    }
    
    /// Load the database for this project.
    pub fn load_database(&self) -> Database {
        Database::load(&self.file_path)
    }
}

/// Convert a display name to a safe project name for file naming.
/// Converts to lowercase and replaces spaces with underscores.
pub fn sanitize_project_name(display_name: &str) -> String {
    display_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c.is_whitespace() {
                '_'
            } else if c == '-' || c == '_' {
                '_'
            } else {
                // Replace other special characters with underscore
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

/// Discover all existing projects in the PM directory.
pub fn discover_projects(pm_dir: &Path) -> Result<Vec<Project>, std::io::Error> {
    let mut projects = Vec::new();
    
    if !pm_dir.exists() {
        return Ok(projects);
    }
    
    for entry in fs::read_dir(pm_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.is_file() {
            if let Some(project) = Project::from_file(path) {
                projects.push(project);
            }
        }
    }
    
    // Sort projects by display name
    projects.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    
    Ok(projects)
}

/// Get the default project (tasks.json) as a special "legacy" project.
pub fn get_legacy_project(pm_dir: &Path) -> Option<Project> {
    let legacy_path = pm_dir.join("tasks.json");
    if legacy_path.exists() {
        Some(Project {
            name: "default".to_string(),
            display_name: "Default (Legacy)".to_string(),
            file_path: legacy_path,
        })
    } else {
        None
    }
}

/// Create a new project with the given name.
pub fn create_project(display_name: &str, pm_dir: &Path) -> Result<Project, Box<dyn std::error::Error>> {
    // Validate project name
    if display_name.trim().is_empty() {
        return Err("Project name cannot be empty".into());
    }
    
    let project = Project::new(display_name, pm_dir);
    
    // Check if project already exists
    if project.file_path.exists() {
        return Err(format!("Project '{}' already exists", display_name).into());
    }
    
    // Create the project database file
    project.create_if_not_exists()?;
    
    Ok(project)
}

/// Find the most recently modified project in the PM directory.
pub fn get_most_recent_project(pm_dir: &Path) -> Result<Option<Project>, std::io::Error> {
    let mut projects = discover_projects(pm_dir)?;
    
    // Add legacy project if it exists
    if let Some(legacy) = get_legacy_project(pm_dir) {
        projects.push(legacy);
    }
    
    if projects.is_empty() {
        return Ok(None);
    }
    
    // Find the project with the most recent modification time
    let mut most_recent: Option<(Project, std::time::SystemTime)> = None;
    
    for project in projects {
        if let Ok(metadata) = fs::metadata(&project.file_path) {
            if let Ok(modified) = metadata.modified() {
                match most_recent {
                    None => most_recent = Some((project, modified)),
                    Some((_, current_time)) => {
                        if modified > current_time {
                            most_recent = Some((project, modified));
                        }
                    }
                }
            }
        }
    }
    
    Ok(most_recent.map(|(project, _)| project))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sanitize_project_name() {
        assert_eq!(sanitize_project_name("My Project"), "my_project");
        assert_eq!(sanitize_project_name("Test-Project_123"), "test_project_123");
        assert_eq!(sanitize_project_name("Special!@#$%Characters"), "special_characters");
        assert_eq!(sanitize_project_name("  Multiple   Spaces  "), "multiple_spaces");
        assert_eq!(sanitize_project_name(""), "");
    }
}