module Main (main) where

import Control.Monad (unless)
import System.Environment (getArgs)
import System.Exit (die)
import Text.Read (readMaybe)
import Test.DocTest.Internal.Extract (Module (Module))
import Test.DocTest.Internal.Location (Located (Located), Location (Location))
import Test.DocTest.Internal.Parse (parseModules)
import Test.DocTest.Internal.Run
  ( Config (ghcOptions, repl)
  , defaultConfig
  , evaluateResult
  , runDocTests
  )

main :: IO ()
main = do
  arguments <- getArgs
  (replCommand, clashArguments, moduleName, sourcePath, documents) <-
    either die pure (parseArguments arguments)
  comments <- mapM readDocument documents
  let config =
        defaultConfig
          { repl = (head replCommand, tail replCommand ++ ["--interactive"])
          , ghcOptions = clashArguments ++ [sourcePath]
          }
      moduleDocs = Module moduleName Nothing comments
  runDocTests config (parseModules [moduleDocs]) >>= evaluateResult

data Document = Document
  { documentSource :: FilePath
  , documentLine :: Int
  , documentPath :: FilePath
  }

readDocument :: Document -> IO (Located String)
readDocument document = do
  contents <- readFile (documentPath document)
  pure (Located (Location (documentSource document) (documentLine document)) contents)

parseArguments :: [String] -> Either String ([String], [String], String, FilePath, [Document])
parseArguments arguments = do
  (replCommand, afterRepl) <- takeCounted "REPL command" arguments
  unless (not (null replCommand)) (Left "mdbook-clash-doctest: the REPL command is empty")
  (clashArguments, remaining) <- takeCounted "Clash arguments" afterRepl
  case remaining of
    moduleName : sourcePath : documentArguments -> do
      documents <- parseDocuments documentArguments
      unless (not (null documents)) (Left "mdbook-clash-doctest: no doctest documents were supplied")
      pure (replCommand, clashArguments, moduleName, sourcePath, documents)
    _ -> Left usage

takeCounted :: String -> [String] -> Either String ([String], [String])
takeCounted label arguments = case arguments of
  countText : remaining -> case readMaybe countText of
    Just count
      | count >= 0 && length remaining >= count -> Right (splitAt count remaining)
    _ -> Left ("mdbook-clash-doctest: invalid " ++ label ++ " count\n" ++ usage)
  [] -> Left usage

parseDocuments :: [String] -> Either String [Document]
parseDocuments [] = Right []
parseDocuments (source : lineText : path : remaining) = do
  line <- maybe (Left ("mdbook-clash-doctest: invalid source line: " ++ lineText)) Right (readMaybe lineText)
  rest <- parseDocuments remaining
  pure (Document source line path : rest)
parseDocuments _ = Left usage

usage :: String
usage =
  "usage: mdbook-clash-doctest REPL_COUNT REPL... CLASH_ARG_COUNT CLASH_ARGS... "
    ++ "MODULE SOURCE (DOCUMENT_SOURCE DOCUMENT_LINE DOCUMENT_PATH)..."
